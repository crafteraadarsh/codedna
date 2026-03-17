# Example: Local Repository Analysis

Analyze a local project directory with CodeDna.

## Basic Analysis

```bash
$ codedna analyze .

╔════════════════════════════════════════╗
║                                        ║
║      CodeDna Intelligence Report      ║
║                                        ║
╚════════════════════════════════════════╝

  Project Type
  ────────────
  Full-stack web application

  Stack
  ─────
  React  +  Vite  +  Express

  Databases
  ─────────
  PostgreSQL,  Redis

  Infrastructure
  ──────────────
  Docker  •  GitHub Actions

  Architecture
  ────────────
  Frontend → API → Database

  Languages
  ─────────
  TypeScript        ████████████████████████░░░░   78%     12,450 LOC
  JavaScript        ████████░░░░░░░░░░░░░░░░░░░░   13%      2,100 LOC
  CSS               ███░░░░░░░░░░░░░░░░░░░░░░░░░    7%      1,100 LOC
  Markdown          █░░░░░░░░░░░░░░░░░░░░░░░░░░░    2%        130 LOC

  Top 5 Files by LOC
  ──────────────────
  1  src/server.ts                                       340 LOC   TypeScript
  2  src/controllers/user.ts                             290 LOC   TypeScript
  3  src/api/routes.ts                                   240 LOC   TypeScript
  4  src/components/Dashboard.tsx                        180 LOC   TypeScript
  5  src/utils/auth.ts                                   160 LOC   TypeScript

  Dead Code
  ─────────
  src/utils/oldHelper.ts
  src/components/LegacyNavbar.tsx

  Summary
  ───────
  Total LOC          15,780
  Languages          4
  Frameworks         3
  Databases          2
  Infrastructure     2
  Files scanned      148
  Dead files         2
  Dependency links   87
```

## JSON Output for Agents

```bash
$ codedna json . --compact | jq -c '{project_type,frameworks,databases,architecture,total_loc}'
{"project_type":"Full-stack web application","frameworks":["React","Vite","Express"],"databases":["PostgreSQL","Redis"],"architecture":"Frontend → API → Database","total_loc":15780}
```

## Targeted Commands

```bash
# Tech stack only
$ codedna stack .

# Files that use React
$ codedna framework react .

# Dead code detection
$ codedna deadcode .

# Directory tree (3 levels deep)
$ codedna map . --depth 3
```

## With Timing

```bash
$ codedna analyze . --time
# ... full report ...
# Elapsed: 2.6ms
```
