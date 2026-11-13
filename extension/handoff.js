// The browser and desktop cannot share one transaction. Only non-secret
// recovery metadata is stored here; the request itself stays in the host's
// encrypted database after prepare.
class Handoff {
  constructor({native, storage, downloads, uuid, onPending = () => {}}) {
    this.native = native;
    this.storage = storage;
    this.downloads = downloads;
    this.uuid = uuid;
    this.onPending = onPending;
    this.inFlight = new Set();
  }

  async call(message) {
    try { return await this.native(message); } catch { return {ok:false}; }
  }

  async save(key, entry) { await this.storage.set({[key]:entry}); }

  async abort(key, entry) {
    const result = await this.call({type:'abort',id:entry.id});
    if (result?.ok) await this.storage.remove(key);
    else this.onPending();
  }

  async commit(key, entry) {
    const result = await this.call({type:'commit',id:entry.id});
    if (!result?.ok || !['queued','running','paused','completed','failed'].includes(result.state)) {
      this.onPending();
      return false;
    }
    // Erasing removes only the browser's interrupted record, not the file.
    if (entry.downloadId !== undefined) {
      try { await this.downloads.erase({id:entry.downloadId}); } catch {}
    }
    await this.storage.remove(key);
    return true;
  }

  async transfer(downloadId, task, policy = 'immediate') {
    const key = `ndmNextHandoff:${downloadId}`;
    if (this.inFlight.has(key)) return false;
    this.inFlight.add(key);
    let entry = {id:this.uuid(),downloadId,phase:'preparing'};
    try {
      const existing = await this.storage.get(key);
      if (existing[key]) { this.onPending(); return false; }
      await this.save(key, entry);
      const prepared = await this.call({type:'prepare',id:entry.id,task});
      if (!prepared?.ok || prepared.state !== 'prepared') {
        await this.abort(key,entry);
        return false;
      }
      entry = {...entry,phase:'prepared'};
      try { await this.save(key,entry); } catch { await this.abort(key,entry); return false; }
      const [current] = await this.downloads.search({id:downloadId});
      if (!current || current.state !== 'in_progress') {
        await this.abort(key,entry);
        return false;
      }
      if (policy !== 'immediate') {
        entry = {...entry,phase:'safe_committing'};
        try { await this.save(key,entry); } catch { await this.abort(key,entry); return false; }
        return await this.startSafe(key,entry);
      }
      entry = {...entry,phase:'cancel_requested'};
      try { await this.save(key,entry); } catch { await this.abort(key,entry); return false; }
      try {
        await this.downloads.cancel(downloadId);
      } catch {
        const [after] = await this.downloads.search({id:downloadId});
        if (after?.state === 'interrupted' && after.error === 'USER_CANCELED') return await this.commit(key,entry);
        if (after && ['in_progress','complete'].includes(after.state)) await this.abort(key,entry);
        else this.onPending();
        return false;
      }
      return await this.commit(key,entry);
    } catch {
      // The last durable phase guides recovery; never cancel if its write failed.
      this.onPending();
      return false;
    } finally {
      this.inFlight.delete(key);
    }
  }

  async discard(key, entry) {
    await this.save(key,{...entry,phase:'safe_discarding'});
    const result = await this.call({type:'discard',id:entry.id});
    if (result?.ok) await this.storage.remove(key);
    else this.onPending();
  }

  async startSafe(key, entry) {
    const [current] = await this.downloads.search({id:entry.downloadId});
    if (!current || current.state !== 'in_progress') {
      await this.discard(key,entry);
      return false;
    }
    const result = await this.call({type:'commit',id:entry.id});
    if (!result?.ok || !['queued','running','paused','completed','failed'].includes(result.state)) {
      this.onPending();
      return false;
    }
    entry = {...entry,phase:'safe_monitor'};
    await this.save(key,entry);
    await this.monitorSafe(key,entry);
    return true;
  }

  async monitorSafe(key, entry) {
    const [current] = await this.downloads.search({id:entry.downloadId});
    if (!current || current.state === 'complete' || (current.state === 'interrupted' && current.error === 'USER_CANCELED' && entry.phase !== 'safe_cancelling')) {
      await this.discard(key,entry);
      return;
    }
    if (current.state === 'interrupted' && current.error !== 'USER_CANCELED') {
      // The browser failed independently; keep the desktop task as a recovery option.
      await this.storage.remove(key);
      return;
    }
    const result = await this.call({type:'inspect',id:entry.id});
    if (!result?.ok) { this.onPending(); return; }
    if (['missing','failed','paused'].includes(result.state)) { await this.discard(key,entry); return; }
    const verified = result.state === 'completed' && result.size_verified === true
      && Number.isSafeInteger(result.output_size) && Number.isSafeInteger(current.totalBytes)
      && current.totalBytes >= 0 && result.output_size === current.totalBytes;
    if (!verified) { this.onPending(); return; }
    if (current.state === 'interrupted' && current.error === 'USER_CANCELED') {
      try { await this.downloads.erase({id:entry.downloadId}); } catch {}
      await this.storage.remove(key);
      return;
    }
    if (current.state !== 'in_progress') { this.onPending(); return; }
    entry = {...entry,phase:'safe_cancelling'};
    await this.save(key,entry);
    try { await this.downloads.cancel(entry.downloadId); } catch {}
    const [after] = await this.downloads.search({id:entry.downloadId});
    if (after?.state === 'interrupted' && after.error === 'USER_CANCELED') {
      try { await this.downloads.erase({id:entry.downloadId}); } catch {}
      await this.storage.remove(key);
    } else if (after?.state === 'complete') await this.discard(key,entry);
    else this.onPending();
  }

  async send(task) {
    const id = this.uuid();
    const key = `ndmNextManual:${id}`;
    const entry = {id,phase:'manual_preparing'};
    this.inFlight.add(key);
    try {
      await this.save(key,entry);
      const prepared = await this.call({type:'prepare',id,task});
      if (!prepared?.ok || prepared.state !== 'prepared') {
        await this.abort(key,entry);
        return false;
      }
      return await this.commit(key,entry);
    } catch {
      this.onPending();
      return false;
    } finally { this.inFlight.delete(key); }
  }

  async recover() {
    const all = await this.storage.get(null);
    let pending = false;
    for (const [key,entry] of Object.entries(all)) {
      if ((!key.startsWith('ndmNextHandoff:') && !key.startsWith('ndmNextManual:')) || !entry || this.inFlight.has(key)) continue;
      this.inFlight.add(key);
      try {
        if (entry.phase === 'manual_preparing') {
          const result = await this.call({type:'inspect',id:entry.id});
          if (!result?.ok) pending = true;
          else if (result.state === 'missing') await this.storage.remove(key);
          else await this.commit(key,entry);
        } else if (entry.phase === 'preparing' || entry.phase === 'prepared') {
          await this.abort(key,entry);
        } else if (entry.phase === 'cancel_requested') {
          const [current] = await this.downloads.search({id:entry.downloadId});
          if (current?.state === 'interrupted' && current.error === 'USER_CANCELED') await this.commit(key,entry);
          else if (current && ['in_progress','complete'].includes(current.state)) await this.abort(key,entry);
          else pending = true;
        } else if (entry.phase === 'safe_committing') {
          await this.startSafe(key,entry);
        } else if (entry.phase === 'safe_monitor' || entry.phase === 'safe_cancelling') {
          await this.monitorSafe(key,entry);
        } else if (entry.phase === 'safe_discarding') {
          await this.discard(key,entry);
        } else pending = true;
      } catch { pending = true; } finally { this.inFlight.delete(key); }
    }
    if (pending) this.onPending();
  }
}

if (typeof module !== 'undefined') module.exports = {Handoff};
