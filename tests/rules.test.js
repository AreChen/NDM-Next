const test = require('node:test');
const assert = require('node:assert/strict');
const { shouldSkip, domainMatches, normalizeDomain, encodeTask } = require('../extension/rules.js');
const defaults = { enabled: true, minBytes: 1048576, excludedExtensions: ['data','woff','woff2'], blockedDomains: ['example.com'] };
test('domains match boundary and subdomains', () => { assert.equal(domainMatches('a.example.com','example.com'), true); assert.equal(domainMatches('notexample.com','example.com'), false); assert.equal(normalizeDomain('https://Example.com/path'), 'example.com'); });
test('skip page or destination domain, types, and known small files', () => { for (const item of [{url:'https://example.com/file.zip'}, {url:'https://other.org/f.zip',pageUrl:'https://a.example.com/'},{url:'https://other.org/a.data?x=1'},{url:'https://other.org/a.zip',size:100}]) assert.equal(shouldSkip(item, defaults), true); });
test('unknown size and threshold boundary are eligible', () => { assert.equal(shouldSkip({url:'https://other.org/a.zip'}, defaults), false); assert.equal(shouldSkip({url:'https://other.org/a.zip',size:1048576}, defaults), false); });
test('protocol encodes safe request headers and context', () => { let task=encodeTask({url:'https://x.test/a',pageUrl:'https://x.test/p',cookie:'a=b',size:12}); assert.match(task,/1:GET\r\n2:https:\/\/x.test\/a\r\n/); assert.match(task,/Cookie: a=b\r\n/); assert.match(task,/7:12\r\n/); assert.throws(()=>encodeTask({url:'https://x.test/\r\nCookie: injected'})); });
