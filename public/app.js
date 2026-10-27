'use strict';

// DOM is built with textContent / attributes only (no innerHTML), so user
// supplied names, versions and spec content can never inject markup.

const $ = (selector, root = document) => root.querySelector(selector);

function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value === undefined || value === null || value === false) continue;
    if (key === 'class') node.className = value;
    else if (key === 'text') node.textContent = value;
    else if (key === 'dataset') Object.assign(node.dataset, value);
    else node.setAttribute(key, value === true ? '' : String(value));
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

const SEMVER = /^\d+\.\d+\.\d+$/;
const REQUEST_TIMEOUT_MS = 20000;

const state = {
  projects: [],
  project: null,
  versions: [],
  selectedId: null,
};

// ---------- API ----------

async function api(url, { method = 'GET', body } = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  const headers = { Accept: 'application/json' };
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  let response;
  try {
    response = await fetch(url, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: controller.signal,
      credentials: 'same-origin',
    });
  } catch (error) {
    throw new Error(error.name === 'AbortError' ? 'The request timed out. Try again.' : 'Could not reach SpecSentry. Check your connection.');
  } finally {
    clearTimeout(timer);
  }
  const text = await response.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = null;
  }
  if (!response.ok) throw new Error((data && data.error) || `Request failed (${response.status}).`);
  return data;
}

// ---------- Formatting ----------

function parseDate(value) {
  if (!value) return null;
  // SQLite CURRENT_TIMESTAMP is UTC "YYYY-MM-DD HH:MM:SS".
  const iso = /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(value) ? `${value.replace(' ', 'T')}Z` : value;
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? null : date;
}

const longDate = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });

function timeEl(value) {
  const date = parseDate(value);
  return date ? el('time', { datetime: date.toISOString(), text: longDate.format(date) }) : el('span', { text: '—' });
}

function plural(n, word) {
  return `${n} ${word}${n === 1 ? '' : 's'}`;
}

/** Normalises current and legacy (Node era) severities. */
function severityOf(change) {
  switch (change.severity) {
    case 'breaking': return { key: 'breaking', label: 'Breaking' };
    case 'warning': return { key: 'warning', label: 'Warning' };
    case 'non-breaking': return { key: 'info', label: 'Addition' };
    default: return { key: 'info', label: 'Info' };
  }
}

/** Mirrors the server rule: breaking → major, any change → minor, none → patch. */
function bump(version, changes) {
  if (!SEMVER.test(version || '')) return '1.0.0';
  const [major, minor, patch] = version.split('.').map(Number);
  if (changes.some(c => c.severity === 'breaking')) return `${major + 1}.0.0`;
  if (changes.length) return `${major}.${minor + 1}.0`;
  return `${major}.${minor}.${patch + 1}`;
}

// ---------- Toast ----------

let toastTimer;
function toast(message, tone = 'info') {
  const node = $('#toast');
  node.textContent = message;
  node.dataset.tone = tone;
  node.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => node.classList.remove('show'), 4500);
}

// ---------- Health ----------

async function refreshHealth() {
  const node = $('#health');
  let ok = false;
  try {
    const data = await api('/api/health');
    ok = Boolean(data && data.ok);
  } catch {
    ok = false;
  }
  node.dataset.state = ok ? 'ok' : 'down';
  $('.health-text', node).textContent = ok ? 'All systems operational' : 'API unavailable';
}

// ---------- Loading ----------

function setView(view, message = '') {
  const dashboard = $('#dashboard');
  dashboard.hidden = view !== 'dashboard' && view !== 'loading';
  dashboard.setAttribute('aria-busy', String(view === 'loading'));
  dashboard.classList.toggle('is-loading', view === 'loading');
  $('#empty').hidden = view !== 'empty';
  $('#load-error').hidden = view !== 'error';
  $('.notice-message', $('#load-error')).textContent = message;
  $('#upload').disabled = view !== 'dashboard';
  $('#download-report').disabled = view !== 'dashboard';
}

function readPreference(key) {
  try { return localStorage.getItem(`specsentry-${key}`); } catch { return null; }
}

function writePreference(key, value) {
  try { localStorage.setItem(`specsentry-${key}`, String(value)); } catch { /* storage blocked */ }
}

async function loadProjects(preferredProjectId, preferredVersionId) {
  setView('loading');
  try {
    state.projects = await api('/api/projects');
    if (!state.projects.length) {
      state.project = null;
      renderProjectPicker();
      setView('empty');
      return;
    }
    const wanted = Number(preferredProjectId ?? readPreference('project'));
    state.project = state.projects.find(p => p.id === wanted) || state.projects[0];
    renderProjectPicker();
    await loadVersions(preferredVersionId);
  } catch (error) {
    setView('error', error.message);
  }
}

async function loadVersions(preferredVersionId) {
  state.versions = await api(`/api/projects/${encodeURIComponent(state.project.id)}/versions`);
  state.selectedId = state.versions.some(v => v.id === preferredVersionId)
    ? preferredVersionId
    : (state.versions[0] ? state.versions[0].id : null);
  writePreference('project', state.project.id);
  setView('dashboard');
  render();
}

// ---------- Rendering ----------

function renderProjectPicker() {
  const select = $('#project-select');
  select.replaceChildren(...state.projects.map(p => el('option', {
    value: p.id,
    text: p.name,
    selected: Boolean(state.project && p.id === state.project.id),
  })));
  $('#project-picker').hidden = state.projects.length < 2;
}

function selectedVersion() {
  return state.versions.find(v => v.id === state.selectedId) || state.versions[0] || null;
}

function render() {
  const selected = selectedVersion();
  const latest = state.versions[0];
  const project = state.project;
  if (!project || !selected || !latest) return;

  const previous = state.versions[state.versions.indexOf(selected) + 1] || null;
  // Legacy rows may contain a "neutral" placeholder meaning "no changes".
  const changes = Array.isArray(selected.changeSummary) ? selected.changeSummary.filter(c => c.severity !== 'neutral') : [];
  const warnings = selected.warningCount ?? changes.filter(c => c.severity === 'warning').length;

  $('#project-name').textContent = project.name;
  $('#repo-name').textContent = project.repository || 'OpenAPI contract';
  $('#latest-version').textContent = `v${latest.version}`;

  const badge = $('#risk-badge');
  badge.hidden = false;
  badge.className = `badge ${selected.breakingCount ? 'badge-breaking' : 'badge-safe'}`;
  badge.textContent = selected.breakingCount ? plural(selected.breakingCount, 'breaking change') : 'Compatible';

  $('#breaking-total').textContent = selected.breakingCount;
  $('#warning-total').textContent = warnings;
  $('#added-total').textContent = selected.addedCount;

  if (previous) {
    const suggested = selected.suggestedVersion || bump(previous.version, changes);
    $('#version-hint').textContent = `v${suggested}`;
    $('#version-note').textContent = suggested === selected.version ? 'Matches the released version' : `Released as v${selected.version}`;
  } else {
    $('#version-hint').textContent = `v${selected.version}`;
    $('#version-note').textContent = 'Baseline snapshot';
  }

  $('#change-version').textContent = `v${selected.version}`;
  $('#change-context').replaceChildren(previous ? `Compared with v${previous.version} · ` : 'Baseline snapshot · ', timeEl(selected.createdAt));

  renderChanges(changes, Boolean(previous));
  renderHistory();
}

function renderChanges(changes, hasPrevious) {
  $('#change-list').replaceChildren(...changes.map(change => {
    const severity = severityOf(change);
    return el('tr', { class: 'diff-row' },
      el('td', { 'data-label': 'Severity' }, el('span', { class: `severity severity-${severity.key}`, text: severity.label })),
      el('td', { 'data-label': 'Change', class: 'diff-kind', text: change.kind || change.rule || 'Change' }),
      el('td', { 'data-label': 'Operation / field', class: 'target mono', text: change.target || '—' }),
      el('td', { 'data-label': 'Detail', class: 'detail' },
        el('span', { text: change.detail || '' }),
        change.pointer ? el('code', { class: 'pointer', title: 'JSON pointer', text: change.pointer }) : null));
  }));
  const empty = $('#change-empty');
  $('.diff-table').hidden = changes.length === 0;
  empty.hidden = changes.length !== 0;
  empty.textContent = hasPrevious
    ? 'No contract changes compared with the previous snapshot.'
    : 'This is the baseline snapshot. Upload a new spec to see what changed.';
}

function renderHistory() {
  const last = state.versions.length - 1;
  $('#version-list').replaceChildren(...state.versions.map((version, i) => {
    const tags = [];
    if (version.breakingCount) tags.push(el('span', { class: 'tag tag-breaking', text: `${version.breakingCount} breaking` }));
    if (version.warningCount) tags.push(el('span', { class: 'tag tag-warning', text: plural(version.warningCount, 'warning') }));
    if (version.addedCount) tags.push(el('span', { class: 'tag tag-info', text: `${version.addedCount} added` }));
    if (i === last) tags.push(el('span', { class: 'tag tag-neutral', text: 'baseline' }));
    else if (!tags.length) tags.push(el('span', { class: 'tag tag-neutral', text: 'no changes' }));
    return el('li', {},
      el('button', {
        type: 'button',
        class: 'version-row',
        'aria-pressed': String(version.id === state.selectedId),
        dataset: { version: version.id },
      },
      el('span', { class: 'version-tag mono', text: `v${version.version}` }),
      el('span', { class: 'version-label', text: i === 0 ? 'Latest contract' : 'Snapshot' }),
      el('span', { class: 'version-time' }, timeEl(version.createdAt)),
      el('span', { class: 'version-summary' }, tags)));
  }));
  $('#version-count').textContent = plural(state.versions.length, 'snapshot');
}

// ---------- Dialog ----------

const dialog = $('#spec-dialog');
const form = $('#spec-form');
let dialogMode = null;
let opener = null;
let submitting = false;

const sampleSpec = {
  openapi: '3.0.3',
  info: { title: 'Payments API', version: '1.0.0' },
  paths: { '/charges': { get: { summary: 'List charges', responses: { 200: { description: 'Charge collection' } } } } },
};

function openDialog(mode, starter, trigger) {
  dialogMode = mode;
  opener = trigger || document.activeElement;
  const isUpload = mode === 'upload';
  form.reset();
  $('#form-error').textContent = '';
  $('#dialog-title').textContent = isUpload ? 'Upload a new snapshot' : 'Connect an API';
  $('#dialog-subtitle').textContent = isUpload
    ? `Paste an OpenAPI JSON document. It is compared with v${state.versions[0].version}, the latest snapshot of ${state.project.name}.`
    : 'Start with a baseline contract. The next snapshot will be compared against it.';
  $('.project-fields', form).hidden = isUpload;
  $('#field-name').required = !isUpload;
  $('#field-name').value = isUpload ? '' : 'Payments API';
  $('#field-version-label').textContent = isUpload ? 'Version (leave blank to use the suggestion)' : 'Baseline version';
  $('#field-version').placeholder = isUpload ? 'e.g. 2.0.0' : '1.0.0';
  $('#field-version-hint').textContent = 'Semantic version: major.minor.patch';
  $('#form-submit').textContent = isUpload ? 'Compare and save snapshot' : 'Create API project';
  $('#field-spec').value = JSON.stringify(starter, null, 2);
  dialog.showModal();
  (isUpload ? $('#field-spec') : $('#field-name')).focus();
}

dialog.addEventListener('close', () => {
  // Fall back to the main region when the opener disappeared (e.g. empty state).
  const target = opener && document.contains(opener) && opener.offsetParent !== null ? opener : $('#main');
  target.focus();
});
dialog.addEventListener('cancel', event => {
  if (submitting) event.preventDefault();
});
dialog.addEventListener('click', event => {
  // A click on the dialog element itself is a click on the backdrop.
  if (!submitting && (event.target === dialog || event.target.closest('[data-close]'))) dialog.close();
});

$('#field-file').addEventListener('change', async event => {
  const file = event.target.files && event.target.files[0];
  if (!file) return;
  if (file.size > 2 * 1024 * 1024) {
    $('#form-error').textContent = 'That file is larger than 2 MB.';
  } else {
    $('#field-spec').value = await file.text();
    $('#form-error').textContent = '';
  }
  event.target.value = '';
});

function readForm() {
  const data = new FormData(form);
  const text = String(data.get('spec') || '').trim();
  const version = String(data.get('version') || '').trim();
  if (!text) throw new Error('Paste an OpenAPI JSON document.');
  let spec;
  try {
    spec = JSON.parse(text);
  } catch (error) {
    throw new Error(`The OpenAPI JSON is not valid: ${error.message}`);
  }
  const isObject = value => Boolean(value) && typeof value === 'object' && !Array.isArray(value);
  if (!isObject(spec) || typeof spec.openapi !== 'string' || !isObject(spec.info) || !isObject(spec.paths)) {
    throw new Error('The document needs openapi, info and paths fields.');
  }
  if (version && !SEMVER.test(version)) throw new Error('Version must use the format major.minor.patch.');
  if (dialogMode === 'create') {
    const name = String(data.get('name') || '').trim();
    if (name.length < 2 || name.length > 80) throw new Error('Enter a project name between 2 and 80 characters.');
    return { name, repository: String(data.get('repository') || '').trim(), version: version || undefined, spec };
  }
  return { version: version || undefined, spec };
}

async function submitForm(payload) {
  if (dialogMode === 'upload') {
    const result = await api(`/api/projects/${encodeURIComponent(state.project.id)}/specs`, { method: 'POST', body: payload });
    return {
      projectId: state.project.id,
      versionId: result.id,
      message: result.breakingCount
        ? `${plural(result.breakingCount, 'breaking change')} detected in v${result.version}. Suggested version: v${result.suggestedVersion}.`
        : `v${result.version} saved with no breaking changes.`,
      tone: result.breakingCount ? 'danger' : 'success',
    };
  }
  const made = await api('/api/projects', { method: 'POST', body: payload });
  return { projectId: made.id, versionId: made.versionId, message: 'Baseline contract connected.', tone: 'success' };
}

form.addEventListener('submit', async event => {
  event.preventDefault();
  if (submitting) return;
  const submit = $('#form-submit');
  const error = $('#form-error');
  error.textContent = '';
  let payload;
  try {
    payload = readForm();
  } catch (problem) {
    error.textContent = problem.message;
    return;
  }
  submitting = true;
  submit.disabled = true;
  form.setAttribute('aria-busy', 'true');
  const label = submit.textContent;
  submit.textContent = dialogMode === 'upload' ? 'Comparing…' : 'Creating…';
  let outcome = null;
  try {
    outcome = await submitForm(payload);
  } catch (problem) {
    error.textContent = problem.message;
  } finally {
    submitting = false;
    submit.disabled = false;
    submit.textContent = label;
    form.removeAttribute('aria-busy');
  }
  if (outcome) {
    dialog.close();
    await loadProjects(outcome.projectId, outcome.versionId);
    toast(outcome.message, outcome.tone);
  }
});

// ---------- Actions ----------

let preparingUpload = false;
$('#upload').addEventListener('click', async event => {
  if (!state.project || !state.versions[0] || preparingUpload) return;
  // aria-busy instead of disabled: disabling would drop focus to <body>.
  const button = event.currentTarget;
  preparingUpload = true;
  button.setAttribute('aria-busy', 'true');
  try {
    const latest = await api(`/api/projects/${encodeURIComponent(state.project.id)}/versions/${encodeURIComponent(state.versions[0].id)}/spec`);
    openDialog('upload', latest.spec, button);
  } catch (error) {
    toast(error.message, 'danger');
  } finally {
    preparingUpload = false;
    button.removeAttribute('aria-busy');
  }
});

$('#new-project').addEventListener('click', event => openDialog('create', sampleSpec, event.currentTarget));
$('#empty-create').addEventListener('click', event => openDialog('create', sampleSpec, event.currentTarget));
$('#retry').addEventListener('click', () => loadProjects());

$('#project-select').addEventListener('change', async event => {
  const id = Number(event.target.value);
  state.project = state.projects.find(p => p.id === id) || state.project;
  setView('loading');
  try {
    await loadVersions();
  } catch (error) {
    setView('error', error.message);
  }
});

$('#version-list').addEventListener('click', event => {
  const row = event.target.closest('[data-version]');
  if (!row) return;
  state.selectedId = Number(row.dataset.version);
  render();
  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  $('#changes').scrollIntoView({ behavior: reduced ? 'auto' : 'smooth', block: 'start' });
});

// Spreadsheets evaluate cells starting with these characters as formulas.
function csvCell(value) {
  let text = String(value ?? '');
  if (/^[=+\-@\t\r]/.test(text)) text = `'${text}`;
  return `"${text.replaceAll('"', '""')}"`;
}

$('#download-report').addEventListener('click', () => {
  const version = selectedVersion();
  if (!version) return;
  const rows = [
    ['severity', 'rule', 'change', 'target', 'detail', 'pointer'],
    ...(version.changeSummary || []).map(c => [c.severity, c.rule, c.kind, c.target, c.detail, c.pointer]),
  ];
  const csv = rows.map(row => row.map(csvCell).join(',')).join('\r\n');
  const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv;charset=utf-8' }));
  const link = el('a', { href: url, download: `specsentry-v${version.version.replace(/[^\w.-]/g, '_')}-change-report.csv` });
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 0);
});

// ---------- Navigation & theme ----------

for (const link of document.querySelectorAll('.nav-link')) {
  link.addEventListener('click', () => {
    for (const other of document.querySelectorAll('.nav-link')) other.removeAttribute('aria-current');
    link.setAttribute('aria-current', 'location');
  });
}

const darkQuery = matchMedia('(prefers-color-scheme: dark)');

function effectiveTheme() {
  return document.documentElement.dataset.theme || (darkQuery.matches ? 'dark' : 'light');
}

function syncThemeButton() {
  const dark = effectiveTheme() === 'dark';
  const button = $('#theme-toggle');
  button.setAttribute('aria-pressed', String(dark));
  button.title = dark ? 'Switch to light mode' : 'Switch to dark mode';
}

$('#theme-toggle').addEventListener('click', () => {
  const next = effectiveTheme() === 'dark' ? 'light' : 'dark';
  const system = darkQuery.matches ? 'dark' : 'light';
  // Picking the system theme drops the override so later OS changes apply again.
  if (next === system) delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = next;
  try {
    if (next === system) localStorage.removeItem('specsentry-theme');
    else localStorage.setItem('specsentry-theme', next);
  } catch { /* storage blocked */ }
  syncThemeButton();
});
darkQuery.addEventListener('change', syncThemeButton);

// ---------- Boot ----------

syncThemeButton();
refreshHealth();
setInterval(refreshHealth, 60000);
loadProjects();
