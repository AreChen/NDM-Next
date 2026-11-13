const test = require('node:test');
const assert = require('node:assert/strict');
const { Handoff } = require('../extension/handoff.js');

function setup({ prepare = {ok:true,state:'prepared'}, commit = {ok:true,state:'queued'}, inspect = {ok:true,state:'running'}, cancelError = false, currentState = 'in_progress', currentError, totalBytes = 2097152 } = {}) {
  const calls = [], records = {}, native = async request => {
    calls.push(request.type);
    if (request.type === 'prepare') return prepare;
    if (request.type === 'commit') return commit;
    if (request.type === 'inspect') return inspect;
    return {ok:true};
  };
  const storage = {
    get: async key => key == null ? {...records} : {[key]:records[key]},
    set: async values => Object.assign(records, values),
    remove: async key => { delete records[key]; },
  };
  const downloads = {
    search: async () => [{id:5,state:currentState,error:currentError,totalBytes}],
    cancel: async () => { calls.push('cancel'); if (cancelError) throw new Error('cancel failed'); currentState='interrupted'; currentError='USER_CANCELED'; },
    erase: async () => { calls.push('erase'); },
  };
  const service = new Handoff({native, storage, downloads, uuid:()=> '00000000-0000-4000-8000-000000000001'});
  const task = {url:'https://files.test/file.zip',headers:{cookie:'private=secret'}};
  return {service, calls, records, task, setCommit:value=>{commit=value},setInspect:value=>{inspect=value},setState:(state,error)=>{currentState=state;currentError=error}};
}

test('persists and acknowledges before cancelling, then commits before erasing', async () => {
  const t=setup();
  assert.equal(await t.service.transfer(5,t.task,'immediate'),true);
  assert.deepEqual(t.calls,['prepare','cancel','commit','erase']);
  assert.deepEqual(t.records,{});
});

test('failed prepare or cancel leaves browser download intact', async () => {
  let t=setup({prepare:{ok:false}});
  assert.equal(await t.service.transfer(5,t.task,'immediate'),false);
  assert.deepEqual(t.calls,['prepare','abort']);
  t=setup({cancelError:true});
  assert.equal(await t.service.transfer(5,t.task,'immediate'),false);
  assert.deepEqual(t.calls,['prepare','cancel','abort']);
});

test('lost commit response is retried after a service worker restart without storing secrets', async () => {
  const t=setup({commit:{ok:false}});
  assert.equal(await t.service.transfer(5,t.task,'immediate'),false);
  assert.deepEqual(t.calls,['prepare','cancel','commit']);
  assert(!JSON.stringify(t.records).includes('private=secret'));
  assert.equal(Object.values(t.records)[0].phase,'cancel_requested');
  t.setCommit({ok:true,state:'queued'});
  await t.service.recover();
  assert.deepEqual(t.calls,['prepare','cancel','commit','commit','erase']);
  assert.deepEqual(t.records,{});
});

test('recovery aborts an unconfirmed task while the browser is still downloading', async () => {
  const t=setup();
  t.records['ndmNextHandoff:5']={id:'00000000-0000-4000-8000-000000000001',downloadId:5,phase:'prepared'};
  await t.service.recover();
  assert.deepEqual(t.calls,['abort']);
  assert.deepEqual(t.records,{});
});

test('recovery commits only a browser cancellation and retains uncertain interrupted states', async () => {
  const t=setup({currentState:'interrupted',currentError:'NETWORK_FAILED'});
  t.records['ndmNextHandoff:5']={id:'00000000-0000-4000-8000-000000000001',downloadId:5,phase:'cancel_requested'};
  await t.service.recover();
  assert.deepEqual(t.calls,[]);
  assert.equal(Object.keys(t.records).length,1);
  t.setState('interrupted','USER_CANCELED');
  await t.service.recover();
  assert.deepEqual(t.calls,['commit','erase']);
  assert.deepEqual(t.records,{});
});

test('browser completion before cancellation aborts the prepared task', async () => {
  const t=setup({currentState:'complete'});
  assert.equal(await t.service.transfer(5,t.task,'immediate'),false);
  assert.deepEqual(t.calls,['prepare','abort']);
});

test('manual media handoff commits without cancelling a browser download', async () => {
  const t=setup();
  assert.equal(await t.service.send(t.task),true);
  assert.deepEqual(t.calls,['prepare','commit']);
  assert.deepEqual(t.records,{});
});

test('safe mode waits for a verified desktop file before cancelling the browser', async () => {
  const t=setup();
  assert.equal(await t.service.transfer(5,t.task),true);
  assert.deepEqual(t.calls,['prepare','commit','inspect']);
  assert.equal(Object.values(t.records)[0].phase,'safe_monitor');
  assert(!JSON.stringify(t.records).includes('private=secret'));
  t.setInspect({ok:true,state:'completed',size_verified:true,output_size:2097152});
  await t.service.recover();
  assert.deepEqual(t.calls,['prepare','commit','inspect','inspect','cancel','erase']);
  assert.deepEqual(t.records,{});
});

test('a failed one-time desktop link never cancels the browser', async () => {
  const t=setup({inspect:{ok:true,state:'failed'}});
  await t.service.transfer(5,t.task);
  assert.deepEqual(t.calls,['prepare','commit','inspect','discard']);
  assert.deepEqual(t.records,{});
});

test('browser finishing first stops the redundant desktop copy', async () => {
  const t=setup(); await t.service.transfer(5,t.task);
  t.setState('complete'); await t.service.recover();
  assert.deepEqual(t.calls,['prepare','commit','inspect','discard']);
  assert.deepEqual(t.records,{});
});

test('unknown or mismatched lengths leave the original browser download alone', async () => {
  for (const output_size of [undefined,1,2097153]) {
    const t=setup({inspect:{ok:true,state:'completed',size_verified:true,output_size}});
    await t.service.transfer(5,t.task);
    assert(!t.calls.includes('cancel'));
  }
});

test('safe recovery handles a lost commit acknowledgement without cancelling the browser', async () => {
  const t=setup({commit:{ok:false}});
  assert.equal(await t.service.transfer(5,t.task),false);
  assert.equal(Object.values(t.records)[0].phase,'safe_committing');
  t.setCommit({ok:true,state:'queued'});
  await t.service.recover();
  assert.deepEqual(t.calls,['prepare','commit','commit','inspect']);
  assert(!t.calls.includes('cancel'));
});

test('cancel failure and uncertain sizes retain the browser download for retry', async () => {
  const t=setup({cancelError:true,inspect:{ok:true,state:'completed',size_verified:true,output_size:2097152}});
  await t.service.transfer(5,t.task);
  assert.deepEqual(t.calls,['prepare','commit','inspect','cancel']);
  assert.equal(Object.values(t.records)[0].phase,'safe_cancelling');
  assert(!t.calls.includes('erase'));
});

test('a browser network failure retains the desktop task as a recovery option', async () => {
  const t=setup(); await t.service.transfer(5,t.task);
  t.setState('interrupted','NETWORK_FAILED'); await t.service.recover();
  assert(!t.calls.includes('discard'));
  assert.deepEqual(t.records,{});
});
