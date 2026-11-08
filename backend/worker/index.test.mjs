import test from 'node:test';
import assert from 'node:assert/strict';
import { validate, safeEqual, authorized } from './index.js';

test('keeps valid IDs, class and name', () => {
  const r = validate({ v: 1, devices: [{ ids: ['PCI\\VEN_10EC&DEV_8168&SUBSYS_1'], class: 'NET', name: 'Realtek `GbE`' }] });
  assert.equal(r.length, 1);
  assert.equal(r[0].name, 'Realtek GbE', 'backticks would break the issue code fence');
});
test('drops IDs outside the allowed alphabet and devices left without IDs', () => {
  assert.equal(validate({ v: 1, devices: [{ ids: ['PCI\\VEN_1; rm -rf /', '<script>'] }] }), null);
  assert.deepEqual(validate({ v: 1, devices: [{ ids: ['bad id', 'USB\\VID_04A9&PID_1830'] }] })[0].ids, ['USB\\VID_04A9&PID_1830']);
});
test('rejects wrong shapes and caps the list', () => {
  assert.equal(validate(null), null);
  assert.equal(validate({ v: 2, devices: [] }), null);
  assert.equal(validate({ v: 1, devices: 'x' }), null);
  const many = Array.from({ length: 80 }, (_, i) => ({ ids: [`PCI\\VEN_8086&DEV_${i}`] }));
  assert.equal(validate({ v: 1, devices: many }).length, 50);
});
test('read key check', () => {
  assert.equal(safeEqual('abc', 'abc'), true);
  assert.equal(safeEqual('abc', 'abd'), false);
  assert.equal(safeEqual('abc', 'abcd'), false);
  assert.equal(safeEqual('', ''), false, 'an empty key never authorizes');
  const req = (h) => ({ headers: { get: () => h } });
  assert.equal(authorized(req('Bearer s3cret'), { READ_KEY: 's3cret' }), true);
  assert.equal(authorized(req('Bearer nope'), { READ_KEY: 's3cret' }), false);
  assert.equal(authorized(req(null), { READ_KEY: 's3cret' }), false);
  assert.equal(authorized(req('Bearer x'), {}), false, 'no configured key means nobody gets in');
});