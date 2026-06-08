# Provider Console UI

The UI in `apps/benchmark-ui` is a static provider console served by
`burd-agent serve`.

It follows `SKILL.md`:

- dark technical Burd design;
- `#0A0A0A`, `#080808`, `#111111` surfaces;
- `#262626` and `#2A2A2A` grid/borders;
- sans headings;
- mono labels and data;
- rectangular buttons;
- modular grid;
- no Akash branding, images, text, or proprietary UI.

Tabs:

1. Overview
2. Hardware
3. Benchmarks
4. Jobs/Leases future
5. Earnings
6. Uptime
7. Security
8. Logs
9. Raw Data

Operational buttons:

- `Executar benchmark`: calls `POST /api/v1/benchmark/run`.
- `Gerar relatorio assinado`: calls `POST /api/v1/report/signed`.
- `Rodar challenge mock`: calls `GET /api/v1/challenge/mock`, then
  `POST /api/v1/challenge/run`.

Future UI work:

- authenticated remote provider access;
- marketplace jobs/leases;
- pricing edits;
- alerts/notifications;
- backend verification history;
- benchmark history charts.

