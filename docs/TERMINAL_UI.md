# pgaegis terminal UI

The terminal UI is rendered by the Rust standard-library-only `src/ui.rs` layer. It adds no Python/Node/runtime dependency.

## Database list

```text
╭──────────────────────── PostgreSQL Databases ─────────────────────────╮
│  Database  │ Owner    │ Size     │ Connections │ Status               │
├────────────────────────────────────────────────────────────────────────┤
│  freeyfi   │ freeyfi  │ 12 GB    │ 37          │ ✓ Online             │
│  postgres  │ postgres │ 8 MB     │ 2           │ ✓ Online             │
╰────────────────────────────────────────────────────────────────────────╯
✓ 2 database(s) found
```

## Users

```text
╭──────────────────────── PostgreSQL Users & Roles ─────────────────────╮
│  Role       │ Login │ Superuser │ Create DB │ Replication │ Conn limit│
├────────────────────────────────────────────────────────────────────────┤
│  postgres   │ ✓ Yes │ ✓ Yes     │ ✓ Yes     │ ✓ Yes       │ -1        │
│  freeyfi    │ ✓ Yes │ No        │ No        │ No          │ -1        │
╰────────────────────────────────────────────────────────────────────────╯
```

## Table health

```text
╭────────────────────────── freeyfi / Tables ───────────────────────────╮
│  Schema │ Table    │ Live rows │ Size  │ Dead rows │ Health           │
├────────────────────────────────────────────────────────────────────────┤
│  public │ sessions │ 3820421   │ 18 GB │ 84239     │ ! Vacuum         │
╰────────────────────────────────────────────────────────────────────────╯
```

## Automation/plain output

Colors are enabled only on interactive stdout. To explicitly suppress ANSI styling:

```bash
NO_COLOR=1 pgaegis doctor
```

Passwords and password hashes are deliberately excluded from table/card output.
