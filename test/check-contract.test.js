const { test } = require('node:test');
const assert = require('node:assert/strict');
const { compare } = require('../scripts/check-contract');

test('reports removed routes, responses, and newly required request fields', () => {
  const base = { paths: { '/items': { get: { responses: { 200: {}, 404: {} } }, post: { requestBody: { content: { 'application/json': { schema: { properties: { name: { type: 'string' } } } } } } } } } };
  const candidate = { paths: { '/items': { get: { responses: { 200: {} } }, post: { requestBody: { content: { 'application/json': { schema: { required: ['name'], properties: { name: { type: 'string' } } } } } } } } } };
  assert.equal(compare(base, candidate).length, 2);
});

test('reports safe additive operations as compatible', () => {
  assert.deepEqual(compare({ paths: { '/items': { get: { responses: { 200: {} } } } } }, { paths: { '/items': { get: { responses: { 200: {} } } }, '/health': { get: { responses: { 200: {} } } } } }), []);
});
