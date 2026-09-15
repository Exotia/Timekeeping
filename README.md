# tk

`tk` is a flexitime (Gleitzeit) tracker for the terminal: it records the hours
you work, subtracts the statutory break, compares each day against a daily
target, and carries the difference forward as a running balance. Everything
lives in one SQLite file and one TOML file inside a single folder you can copy
between machines, and the same binary is both a terminal UI and a scriptable
command-line tool.

---

## Install

```bash
# Nix (flake): builds the release binary
nix build
./result/bin/tk

# Cargo, into ~/.cargo/bin
cargo install --path .

# Development shell (cargo, rustc, clippy, rustfmt, rust-analyzer, sqlite)
nix develop
```

## First run

`tk` keeps all of its state in one data directory. On the first run it creates
that directory, writes a default `config.toml` into it, and creates the
database:

```
~/.local/share/tk/
├── config.toml      # written on first run if absent
├── tk.db            # SQLite database
└── backups/         # created by `tk backup`
```

The directory is chosen in this order:

1. `--home DIR` on the command line (works with every subcommand and with the TUI),
2. the `TK_HOME` environment variable, if set and non-empty,
3. `$XDG_DATA_HOME/tk`, i.e. `~/.local/share/tk` on Linux.

```bash
tk                          # open the TUI on the default home
TK_HOME=~/work-hours tk     # a separate set of books
tk --home /tmp/scratch status
```

A broken `config.toml` is fatal: `tk` prints the offending field and exits with
status **2**. Every other error exits with status **1**.

## Configuration

`config.toml` is read once at startup — edit it and restart `tk`. This is the
file written on first run, verbatim:

```toml
# tk configuration — edit and restart tk
start_date = "2026-01-01"          # balance is computed from this date
initial_balance_minutes = 0        # carried-over balance at start_date
daily_target_minutes = 468         # 7:48
vacation_days_per_year = 30
week_starts_on = "monday"          # display only
theme = "dark"                     # "dark" | "light"
extra_holidays = []                # e.g. ["2026-12-24", "2026-12-31"]

[[break_tiers]]                    # ascending; last matching tier applies
after_minutes = 180
deduct_minutes = 18

[[break_tiers]]
after_minutes = 360
deduct_minutes = 48

[theme_overrides]                  # optional; any role may be set to "#rrggbb" or a named color
# positive = "#a6e3a1"
```

| Field | Default | Meaning |
| --- | --- | --- |
| `start_date` | *required* | `YYYY-MM-DD`; the balance sums every day from here to today. Days before it are ignored. |
| `initial_balance_minutes` | `0` | Balance you already carried on `start_date`. Negative values are allowed. |
| `daily_target_minutes` | *required* | Target for one working day; must be greater than 0. `468` is 7:48. |
| `vacation_days_per_year` | `30` | Allowance shown in the title bar and on the statistics screen; remaining = allowance − vacation working days taken in the calendar year. No carry-over. |
| `week_starts_on` | `"monday"` | `"monday"` or `"sunday"`. Accepted and validated, but it has no effect yet: week rows are grouped by ISO week number, which always begins on Monday. |
| `theme` | `"dark"` | `"dark"` or `"light"`. |
| `extra_holidays` | `[]` | Extra `YYYY-MM-DD` dates treated as public holidays on top of the built-in Saxon ones (company holidays such as 24 and 31 December). |
| `[[break_tiers]]` | 180→18, 360→48 | Statutory break table. `after_minutes` must be non-negative and strictly ascending across tiers; `deduct_minutes` must be non-negative. |
| `[theme_overrides]` | empty | Per-role color overrides. |

**How break tiers apply.** For a day's gross time, `tk` picks the tier with the
largest `after_minutes` that is *strictly below* the gross, and subtracts that
tier's `deduct_minutes`. With the defaults, 3:00 gross loses nothing, 3:01
loses 18 minutes, and anything over 6:00 loses 48 minutes. A day with no
entries never gets a deduction.

**Theme overrides.** Keys inside `[theme_overrides]` are role names; values are
`"#rrggbb"` or a named terminal color. Recognised roles: `positive`,
`negative`, `warning`, `accent`, `muted`, `text`, `bg_selected`,
`chip_vacation`, `chip_flex`, `chip_holiday`, `chip_sick`, `chip_absence`. An
unrecognised role name is ignored; an unrecognised *top-level* key is a config
error. On terminals without truecolor support the whole palette is downgraded
to the 256-color cube automatically.

## Day types

Every calendar day has one kind. It decides whether the daily target counts and
whether entries are allowed:

| Kind | Target applies | Entries allowed |
| --- | --- | --- |
| `work` | yes (on weekdays) | yes |
| `flex` | yes — this is how you burn balance for a day off | no |
| `vacation` | no | no |
| `holiday` | no | no |
| `sick` | no | no |
| `absence` (with a free-text `--label`) | no | no |

Weekends never carry a target. A past weekday that is still `work` and has no
entries is flagged `missing` in the month table and does not silently eat your
balance. A public holiday is applied automatically unless you have stored a
kind for that day yourself (so you can set a worked holiday back to `work`).

## Command line

Run `tk` with no subcommand to open the TUI. Everything else is scriptable.

| Command | Options | What it does |
| --- | --- | --- |
| `tk` | | Open the terminal UI. |
| `tk in` | `--force` | Clock in at the current minute. `--force` replaces an existing open clock-in. |
| `tk out` | `-p, --project NAME`<br>`-m, --comment TEXT` | Close the open session and record the entry. Without `--project` the last used project is reused; if there is none, the command fails. |
| `tk status` | | One line for prompts and status bars: running time and today's net while clocked in, otherwise today's net plus the overall balance. |
| `tk add DATE RANGE` | `-p, --project NAME` *(required)*<br>`-m, --comment TEXT` | Add an entry, e.g. `tk add 2026-09-14 0900-1530 -p Alpha -m "review"`. |
| `tk day DATE KIND` | `--to DATE`<br>`--label TEXT` | Set the kind of one day, or of every day from `DATE` to `--to` inclusive. `KIND` is `work`, `vacation`, `flex`, `holiday`, `sick` or `absence`; `--label` names an `absence`. |
| `tk projects` | | Same as `tk projects list`. |
| `tk projects list` | | List projects; archived ones are marked. |
| `tk projects add NAME` | | Create a project. |
| `tk projects archive NAME` | | Hide a project from the picker without deleting its entries. |
| `tk projects unarchive NAME` | | Undo an archive. |
| `tk projects rename OLD NEW` | | Rename a project. |
| `tk backup` | | Copy the database to `<home>/backups/tk-YYYYmmdd-HHMMSS.db`. |
| `tk export` | `--format csv\|json` (default `csv`)<br>`--from DATE`<br>`--to DATE`<br>`-o, --output PATH` | Export entries. Defaults to `start_date` … today, printed to stdout unless `--output` is given. |

Global flags, accepted with any subcommand: `--home DIR`, `--help`, `--version`.

**DATE arguments** accept `YYYY-MM-DD`, the words `today`, `yesterday` and
`tomorrow`, or a whole-number offset in days from today (`-1` is yesterday,
`3` is three days out).

**RANGE** is `START-END`, each side written as `8`, `08`, `800`, `0800`,
`8:00`, or `8.00`. An `END` earlier than `START` means the entry crosses
midnight; `END` equal to `START` is rejected.

```bash
tk in
tk out -p Alpha -m "sprint review"
tk add yesterday 9-1730 -p Alpha
tk day 2026-12-27 vacation --to 2026-12-31
tk day 2026-10-02 absence --label "Betriebsausflug"
tk export --format json --from 2026-01-01 -o hours.json
```

## Terminal UI

The UI needs at least **80×24**; smaller than that it shows only a
"Terminal too small" notice. Below 90 columns the month table drops the comment
column, and below 100 columns the month summary is stacked under the table.

`Ctrl+C` quits from anywhere. `?` opens the key help for the current screen;
any key closes it again.

### Month view (the screen `tk` opens on)

| Key | Action |
| --- | --- |
| `↑` / `k`, `↓` / `j` | Move by day |
| `[` / `PageUp`, `]` / `PageDown` | Previous / next month |
| `t` | Jump to today |
| `Enter` | Open the day editor |
| `s` | Statistics |
| `i` / `o` | Clock in / clock out |
| `v` | Mark the day as vacation |
| `f` | Mark the day as a flex day |
| `x` | Mark the day as sick |
| `p` | Mark the day as a public holiday |
| `w` | Reset the day to a work day |
| `?` | Help |
| `Esc` | Close an open overlay |
| `q`, `Ctrl+C` | Quit |

Changing a day type asks for confirmation first. Clocking in while a session is
already open asks whether to replace it.

### Day editor

| Key | Action |
| --- | --- |
| `↑` / `k`, `↓` / `j` | Select an entry |
| `a` | Add an entry |
| `e`, `Enter` | Edit the selected entry |
| `d`, `Delete` | Delete the selected entry (asks first) |
| `←` / `h`, `→` / `l` | Cycle the day type: work → vacation → flex → holiday → sick → absence |
| `?` | Help |
| `Esc`, `q` | Back to the month view |

Only `work` days may hold entries; saving one on any other kind reports
"change the day type to work first" in the status bar.

### Entry form (overlay)

Four fields in order: **start**, **end**, **project**, **comment**.

| Key | Action |
| --- | --- |
| `Tab`, `↓` | Next field |
| `Shift+Tab`, `↑` | Previous field |
| `↑`, `↓` *(in the project field)* | Move the project picker instead |
| `Tab`, `Enter` *(in the project field)* | Take the highlighted project and jump to the comment |
| `Enter` *(in the comment field)* | Save |
| `Enter` *(any other field)* | Next field |
| `Ctrl+S` | Save from any field |
| `Esc` | Cancel |
| `←`, `→`, `Home`, `End` | Move the cursor in the focused field |
| `Backspace` | Delete the character before the cursor |
| `Delete` | Clear the field |

Typing in the project field filters the picker; a name that matches nothing is
created as a new project on save. The footer validates on every keystroke and
previews this entry's gross time, the day's break deduction, and the day's net.

### Statistics

| Key | Action |
| --- | --- |
| `1` | This month |
| `2` | Last month |
| `3` | This quarter |
| `4` | This year |
| `?` | Help |
| `Esc`, `q` | Back to the month view |

### Overlays

| Overlay | Keys |
| --- | --- |
| Confirm dialog | `y` or `Enter` confirms; `n`, `Esc` or `q` cancels |
| Help | Any key closes it |

## Moving your data around

The data directory is self-contained: `config.toml`, `tk.db` and `backups/`.
Copy the folder to another machine and point `tk` at it, and nothing else is
needed.

```bash
# take it with you
cp -r ~/.local/share/tk /run/media/usb/tk
# use it there
tk --home /run/media/usb/tk
```

```bash
tk backup                      # snapshot into <home>/backups/
tk export --format csv -o hours.csv
tk export --format json --from 2026-01-01 --to 2026-12-31 -o 2026.json
```

The CSV columns are `date,start,end,project,comment,gross`; JSON records carry
the same fields plus `gross_minutes` as an integer.

## Public holidays

`tk` has the public holidays of **Saxony** (as they apply in Dresden) built in,
so holidays never count against your balance and you do not have to enter them:

| Holiday | Date |
| --- | --- |
| Neujahr | 1 January |
| Karfreitag | Easter − 2 |
| Ostermontag | Easter + 1 |
| Tag der Arbeit | 1 May |
| Christi Himmelfahrt | Easter + 39 |
| Pfingstmontag | Easter + 50 |
| Tag der Deutschen Einheit | 3 October |
| Reformationstag | 31 October |
| Buß- und Bettag | Wednesday before 23 November |
| 1. Weihnachtstag | 25 December |
| 2. Weihnachtstag | 26 December |

**Fronleichnam is deliberately not included** — it is a public holiday only in
the designated Sorbian Catholic municipalities of Saxony, and Dresden is not
one of them. Easter is computed with the Gregorian computus (Meeus / Jones /
Butcher), so the movable dates are right for any year.

A holiday falling on a weekend changes nothing. Company holidays and other
local closures go into `extra_holidays` in `config.toml`:

```toml
extra_holidays = ["2026-12-24", "2026-12-31"]
```

If you actually worked on a holiday, set that day back to `work` (`w` in the
month view, or `tk day DATE work`) — a stored day kind always wins over the
computed one.

## Development

```bash
./scripts/check.sh              # rustfmt --check, clippy -D warnings, cargo test
nix develop -c cargo test       # tests only
nix develop -c cargo test --test snapshots   # full-screen TUI snapshots
nix develop -c cargo fmt
```

The layout is `src/core` (pure rules: balance, breaks, holidays, parsing),
`src/store` (SQLite), `src/config`, `src/cli`, and `src/tui` (`model.rs` holds
all state and draws every screen; `view/` renders, `components/` is the key
maps). `Model::draw` is pure over `&self`, which is what lets
`tests/snapshots.rs` render whole screens through a `TestBackend` and assert on
the text.

## License

MIT.
