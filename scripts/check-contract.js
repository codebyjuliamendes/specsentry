#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');

function compare(oldSpec, newSpec) {
  const ops = spec => new Map(Object.entries(spec.paths || {}).flatMap(([route, item]) =>
    Object.entries(item || {})
      .filter(([method]) => ['get','post','put','patch','delete','options','head','trace'].includes(method.toLowerCase()))
      .map(([method, operation]) => [`${method.toUpperCase()} ${route}`, operation])));
  const oldOps = ops(oldSpec), newOps = ops(newSpec), changes = [];
  for (const [key, previous] of oldOps) {
    const next = newOps.get(key);
    if (!next) { changes.push({ severity: 'breaking', target: key, detail: 'Operation was removed.' }); continue; }
    const oldParams = new Map((previous.parameters || []).map(p => [`${p.in}:${p.name}`, p]));
    const nextParams = new Map((next.parameters || []).map(p => [`${p.in}:${p.name}`, p]));
    for (const [name, param] of oldParams) {
      const replacement = nextParams.get(name);
      if (!replacement) changes.push({ severity: 'breaking', target: `${key} · ${name}`, detail: 'Parameter was removed.' });
      else if (param.schema?.type && replacement.schema?.type && param.schema.type !== replacement.schema.type)
        changes.push({ severity: 'breaking', target: `${key} · ${name}`, detail: `Parameter type changed from ${param.schema.type} to ${replacement.schema.type}.` });
      else if (!param.required && replacement.required) changes.push({ severity: 'breaking', target: `${key} · ${name}`, detail: 'Parameter is now required.' });
    }
    for (const code of Object.keys(previous.responses || {})) if (!(code in (next.responses || {})))
      changes.push({ severity: 'breaking', target: `${key} · ${code}`, detail: 'Response code was removed.' });
    const oldBody = previous.requestBody?.content?.['application/json']?.schema;
    const newBody = next.requestBody?.content?.['application/json']?.schema;
    if (oldBody && newBody) {
      const before = new Set(oldBody.required || []);
      for (const field of newBody.required || []) if (!before.has(field)) changes.push({ severity: 'breaking', target: `${key} · ${field}`, detail: 'Request field became required.' });
      for (const field of Object.keys(oldBody.properties || {})) if (!(field in (newBody.properties || {})))
        changes.push({ severity: 'breaking', target: `${key} · ${field}`, detail: 'Request field was removed.' });
    }
  }
  return changes;
}
function read(file) {
  const value = JSON.parse(fs.readFileSync(file, 'utf8'));
  if (!value.openapi || !value.info || !value.paths || typeof value.paths !== 'object') throw new Error(`${file}: expected an OpenAPI document with openapi, info, and paths.`);
  return value;
}
function main(args) {
  const baseIndex = args.indexOf('--base'), candidateIndex = args.indexOf('--candidate');
  if (baseIndex < 0 || candidateIndex < 0 || !args[baseIndex + 1] || !args[candidateIndex + 1]) {
    console.error('Usage: node scripts/check-contract.js --base <openapi.json> --candidate <openapi.json>'); return 2;
  }
  try {
    const base = read(path.resolve(args[baseIndex + 1])), candidate = read(path.resolve(args[candidateIndex + 1]));
    const changes = compare(base, candidate);
    if (!changes.length) { console.log('No breaking contract changes detected.'); return 0; }
    for (const item of changes) console.log(`BREAKING  ${item.target}  ${item.detail}`);
    console.error(`Found ${changes.length} breaking change(s).`); return 1;
  } catch (error) { console.error(error.message); return 2; }
}
if (require.main === module) process.exitCode = main(process.argv.slice(2));
module.exports = { compare, main };
