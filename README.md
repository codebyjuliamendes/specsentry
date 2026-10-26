# SpecSentry

SpecSentry compares OpenAPI JSON snapshots and surfaces potentially breaking contract changes before an API release.

## Run locally

Requires Node.js 20 or newer.

```bash
npm install
npm start
```

Open <http://localhost:3005>. The first run creates a fictional Acme Billing API with two snapshots so the change report is visible immediately. SQLite data lives in `data/specsentry.sqlite` and is not committed.

## Features

- API projects with a GitHub repository reference.
- Immutable OpenAPI JSON snapshots and semantic version labels.
- First-pass checks for removed operations, parameters, and response codes; changed parameter types; and newly required or removed request fields.
- Suggested major or minor version based on detected changes.
- Change report export as CSV.
- Suggested version can be overridden when a team has a different release policy.

## Design notes

- Each snapshot is preserved, so a new comparison does not overwrite the baseline.
- A local CI command compares two OpenAPI JSON documents and fails on detected breaking changes. The included GitHub Actions workflow checks the sample contract and runs the regression tests.
- The analyzer is intentionally conservative and rule-based. It is a useful first review, not a complete OpenAPI compatibility proof. A real release workflow should add reference resolution, response-schema compatibility, test traffic, and team review.
- This MVP accepts OpenAPI JSON. YAML parsing and GitHub App installation are future work.

## API

- `GET /api/projects` — list tracked APIs.
- `POST /api/projects` — add a project and baseline snapshot.
- `GET /api/projects/:id/versions` — list contract snapshots and change summaries.
- `GET /api/projects/:id/versions/:versionId/spec` — read a stored document.
- `POST /api/projects/:id/specs` — compare and store a new snapshot.

## Product direction

The product could grow into a pull-request check for API owners and their SDK or integration consumers. A useful pricing experiment would charge by the number of API contracts monitored, with a free tier for one public spec.\n\n## Run the CI contract check\n\n```bash\nnpm test\nnpm run contract:check -- --base openapi/base.json --candidate openapi/candidate.json\n```\n\nPoint the command at your baseline and candidate OpenAPI JSON files. The check exits nonzero when it finds removed operations, parameters, response codes, or request fields, changed parameter types, or newly required fields. The analyzer is intentionally conservative and does not yet compare response schemas or resolve references.

