const test = require('node:test');
const assert = require('node:assert/strict');
const { captureRequest, findCapture } = require('../extension/capture.js');

const details = overrides => ({requestId:'1',method:'GET',url:'https://files.test/archive.zip',tabId:7,requestHeaders:[
  {name:'Accept',value:'*/*'},{name:'Cookie',value:'session=private'},{name:'Sec-Fetch-Site',value:'same-origin'}
],...overrides});

test('captures only safe replay headers from an observed GET', () => {
  const capture=captureRequest(details());
  assert.equal(capture.url,'https://files.test/archive.zip');
  assert.equal(capture.headers.cookie,'session=private');
  assert.equal(capture.headers.accept,'*/*');
  assert.equal(capture.headers['sec-fetch-site'],undefined);
});

test('rejects non-GET, unknown headers, byte ranges and missing metadata', () => {
  for (const input of [
    details({method:'POST'}), details({requestHeaders:undefined}),
    details({requestHeaders:[{name:'Range',value:'bytes=1-'}]}),
    details({requestHeaders:[{name:'X-Csrf-Token',value:'private'}]}),
    details({url:'https://user:password@files.test/file'}),
  ]) assert.equal(captureRequest(input),null);
});

test('captures signed and token-bearing download URLs', () => {
  for (const url of [
    'https://files.test/file?token=single-use',
    'https://files.test/file?X-Amz-Signature=signed',
  ]) assert.equal(captureRequest(details({url})).url, url);
});

test('matches an unambiguous recent final URL only', () => {
  const now=Date.now();
  const one={...captureRequest(details()),when:now};
  assert.equal(findCapture([one],one.url,now),one);
  assert.equal(findCapture([one,{...one,requestId:'2'}],one.url,now),null);
  assert.equal(findCapture([one],one.url,now+120001),null);
  assert.equal(findCapture([one],'https://other.test/',now),null);
});
