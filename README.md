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
theme = "dark"                     # "dark" | "light" | "purple"
hours_format = "hm"                # "hm" (07:48) | "decimal" (7.80h)
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
| `theme` | `"dark"` | `"dark"`, `"light"` or `"purple"` (deep purple ground, vivid green/red balances). |
| `hours_format` | `"hm"` | How durations are written: `"hm"` is `+07:48`, `"decimal"` is `+7.80h`. Applies to the TUI and to everything `tk` prints except `tk export`, whose `gross` column stays `±HH:MM`. `u` in the TUI flips it and writes the new value back here. |
| `extra_holidays` | `[]` | Extra `YYYY-MM-DD` dates treated as public holidays on top of the built-in Saxon ones (company holidays such as 24 and 31 December). |
| `[[break_tiers]]` | 180→18, 360→48 | Statutory break table, applied to each seamless session on its own length. `after_minutes` must be non-negative and strictly ascending across tiers; `deduct_minutes` must be non-negative. |
| `[theme_overrides]` | empty | Per-role color overrides. |

**Changing your settings.** The four settings you are most likely to revisit —
`start_date`, `initial_balance_minutes`, `daily_target_minutes` and
`vacation_days_per_year` — can be changed without opening an editor, either
with `tk config` or with `c` in the TUI's month view. Both rewrite
`config.toml` in place, keeping your comments, key order and break tiers, and
both refuse a value the config file itself would reject: `tk config` names the
offending field and exits with status 1, the overlay puts the reason in its
footer and stays open. `hours_format` is the fifth: `tk config --hours` sets
it, and `u` in the TUI toggles it on the spot.

```bash
tk config                                   # show the current values
tk config --target 8:00 --vacation 28       # change two of them
tk config --start 2026-01-01 --balance -2:30
tk config --hours decimal                   # print 7.80h instead of 07:48
```

**How breaks apply.** The deduction is charged per *session*, not per day. Your
entries are grouped into sessions: entries that touch or overlap are one
session — a project switch at noon is not a break — and any gap of a minute or
more starts a new one. Each session is then charged on its own length: `tk`
picks the tier with the largest `after_minutes` *strictly below* that session's
length and subtracts its `deduct_minutes`, and the day's deduction is the sum.
With the defaults, 3:00 loses nothing, 3:01 loses 18 minutes and anything over
6:00 loses 48. So 08:00–12:00 plus 12:00–17:00 is one nine-hour session and
loses 48 minutes (net 8:12), while the same hours with a 45-minute lunch —
08:00–12:00 plus 12:45–17:00 — are two sessions of 4:00 and 4:15 and lose 18
each (net 7:39). Take a proper lunch and your afternoon starts a fresh session,
so a long day charged once for being over six hours becomes two shorter ones.
A day with no entries never gets a deduction. While you are clocked in, the
running session counts too, so `tk status` shows the same figures the day will
have once you clock out.

**Project time is net time.** A session's deduction belongs to the session, not
to any one entry of it, so it is shared out over that session's entries in
proportion to their gross length, rounded to whole minutes so the shares add up
to the deduction exactly. An entry's net is its gross minus its share. With the
defaults, 08:00–12:00 on Alpha plus 12:00–17:00 on Beta is one nine-hour session
losing 48 minutes: 21 of them come off Alpha's four hours and 27 off Beta's
five, so Alpha nets 3:39, Beta 4:33 and the day 8:12. Every project figure `tk`
shows — the month summary, the statistics panel, the day editor and the export —
is net time, so the hours on your projects add up to the hours on your balance
and a pause is never booked on a project.

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
| `tk in` | `-p, --project NAME`<br>`--force` | Clock in at the current minute on `NAME`. Without `--project` the last used project is taken; on a brand-new database there is none and the command fails. `--force` replaces an existing open clock-in. |
| `tk switch` | `-p, --project NAME` *(required)*<br>`-m, --comment TEXT` | Record the running session and clock in on `NAME` in one step. The new session starts exactly where the recorded entry ends, so the two never overlap. Switching to the project already running is refused. |
| `tk out` | `-p, --project NAME`<br>`-m, --comment TEXT` | Close the open session and record the entry on the project it was opened with. `--project` books it on another one instead, which is how a clock-in on the wrong project is corrected. |
| `tk status` | | One line for prompts and status bars. Clocked in: the project, the running time, the time you clocked in at, today's net and the overall balance. Otherwise: `not clocked in`, today's net and the overall balance. |
| `tk add DATE RANGE` | `-p, --project NAME` *(required)*<br>`-m, --comment TEXT` | Add an entry, e.g. `tk add 2026-09-14 0900-1530 -p Alpha -m "review"`. |
| `tk day DATE KIND` | `--to DATE`<br>`--label TEXT` | Set the kind of one day, or of every day from `DATE` to `--to` inclusive. `KIND` is `work`, `vacation`, `flex`, `holiday`, `sick` or `absence`; `--label` names an `absence`. |
| `tk projects` | | Same as `tk projects list`. |
| `tk projects list` | | List projects; archived ones are marked. |
| `tk projects add NAME` | | Create a project. |
| `tk projects archive NAME` | | Hide a project from the picker without deleting its entries. |
| `tk projects unarchive NAME` | | Undo an archive. |
| `tk projects rename OLD NEW` | | Rename a project. |
| `tk config` | `--start DATE`<br>`--balance ±HH:MM`<br>`--target HH:MM`<br>`--vacation DAYS`<br>`--hours hm\|decimal` | Without flags, print the five core settings. With any flag, change them in `config.toml` — comments and break tiers are kept — and print the new table. A running TUI keeps the old values until it is restarted, except `hours_format`, which `u` changes live. |
| `tk backup` | | Copy the database to `<home>/backups/tk-YYYYmmdd-HHMMSS.db`. |
| `tk export` | `--format csv\|json` (default `csv`)<br>`--from DATE`<br>`--to DATE`<br>`-o, --output PATH` | Export entries. Defaults to `start_date` … today, printed to stdout unless `--output` is given. |

Global flags, accepted with any subcommand: `--home DIR`, `--help`, `--version`.
`tk help <subcommand>` prints the help for one subcommand, the same as
`tk <subcommand> --help`.

**DATE arguments** accept `YYYY-MM-DD`, the words `today`, `yesterday` and
`tomorrow`, or a whole-number offset in days from today (`-1` is yesterday,
`3` is three days out).

**RANGE** is `START-END`, each side written as `8`, `08`, `800`, `0800`,
`8:00`, or `8.00`. An `END` earlier than `START` means the entry crosses
midnight; `END` equal to `START` is rejected.

```bash
tk in -p Alpha
tk switch -p Beta -m "sprint review"
tk out -m "wrap-up"
tk add yesterday 9-1730 -p Alpha
tk day 2026-12-27 vacation --to 2026-12-31
tk day 2026-10-02 absence --label "Betriebsausflug"
tk export --format json --from 2026-01-01 -o hours.json
```

## Terminal UI

The UI needs at least **80×24**; smaller than that it shows only a
"Terminal too small" notice. Below 90 columns the month table drops the comment
column, and below 92 the one-line key-hint row at the bottom drops the day-type
keys, which no longer fit — `?` still lists them. The month summary always sits
below the table; below 100 columns its height is clamped to at most a third of
the space the two share, so the table keeps most of the room.

`Ctrl+C` quits from anywhere. `?` opens the key help for the current screen;
any key closes it again.

`u` switches every duration on screen between `h:mm` (`+07:48`) and decimal
hours (`+7.80h`) and writes the choice to `hours_format` in `config.toml`, so
the next run — and `tk status` in your prompt — opens the way you left it. It
works on all three screens, whenever no overlay has the keyboard. The month
screen's hint row has no room for it at 80 columns; `?` lists it there.

### Month view (the screen `tk` opens on)

| Key | Action |
| --- | --- |
| `↑` / `k`, `↓` / `j` | Move by day |
| `[` / `PageUp`, `]` / `PageDown` | Previous / next month |
| `t` | Jump to today |
| `Enter` | Open the day editor |
| `s` | Statistics |
| `c` | Settings overlay: start date, initial balance, daily target, vacation days |
| `i` | Clock in on a project — while clocked in, switch to another one |
| `o` | Clock out |
| `v` | Mark the day as vacation |
| `f` | Mark the day as a flex day |
| `x` | Mark the day as sick |
| `p` | Mark the day as a public holiday |
| `w` | Reset the day to a work day |
| `u` | Toggle every duration between `h:mm` and decimal hours |
| `?` | Help |
| `Esc` | Close an open overlay |
| `q`, `Ctrl+C` | Quit |

Changing a day type asks for confirmation first, and is refused with a status
message if the day already has entries, or if it is a weekend (weekends never
carry a target, so they need no day type).

`i` opens a one-field project picker: type to filter the known projects, `↑`
and `↓` to move the highlight, `Enter` or `Tab` to take it, `Esc` to cancel. A
name that matches nothing is created as a new project. With nothing running the
picker is titled "Clock in — project" and offers the last used project first;
while clocked in it is titled "Switch project — currently NAME", leaves that project
out, and choosing one records the running session and clocks in on the new
project in a single step. The title bar names the project it is counting for:
`⏱ Alpha in since 08:12 (03:41)`.

### Day editor

| Key | Action |
| --- | --- |
| `↑` / `k`, `↓` / `j` | Select an entry |
| `a` | Add an entry |
| `e`, `Enter` | Edit the selected entry |
| `d`, `Delete` | Delete the selected entry (asks first) |
| `←` / `h`, `→` / `l` | Cycle the day type: work → vacation → flex → holiday → sick → absence |
| `u` | Toggle every duration between `h:mm` and decimal hours |
| `?` | Help |
| `Esc`, `q` | Back to the month view |

Only `work` days may hold entries; saving one on any other kind reports
"change the day type to work first" in the status bar. Cycling the day type is
itself refused — with "Day has entries; delete them first" or "Weekends need no
day type" — while the day holds entries or falls on a weekend.

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
| `Delete` | Delete the character under the cursor |

Typing in the project field filters the picker; a name that matches nothing is
created as a new project on save. The footer validates on every keystroke and
previews this entry's gross time, the day's break deduction, and the day's net.

### Settings (overlay)

`c` on the month view opens the same four settings `tk config` changes, in a
form: **start date**, **initial balance**, **daily target**, **vacation days**.
The keys are the entry form's — `Tab` / `Shift+Tab` and `Enter` move between
fields, `Ctrl+S` or `Enter` on the last field saves, `Esc` cancels.

The footer validates on every keystroke and previews what saving would set. On
save `tk` rewrites `config.toml` (comments and break tiers intact) and applies
the new rules straight away: balances are recomputed from the new start date,
target and initial balance without restarting. A value the config file would
reject keeps the overlay open with the reason in the footer. The settings not
in this form — theme, extra holidays, break tiers — still need the file and a
restart.

### Statistics

| Key | Action |
| --- | --- |
| `1` | This week (Monday … Sunday) |
| `2` | This month |
| `3` | Last month |
| `4` | This quarter |
| `5` | This year |
| `u` | Toggle every duration between `h:mm` and decimal hours |
| `?` | Help |
| `Esc`, `q` | Back to the month view |

Under the range selector sits the overtime chart: one bar per day for a week,
per ISO week for a month, per month for a quarter or a year. Bars grow to the
right of the zero line when the period is over its target and to the left when
it is under it, and the line below them sums the range and names its best and
worst period. A second line under it carries `net`, `target` and `balance` for
the range: a target and a balance are about your days, not about your projects,
so they are only ever shown next to the chart they move. Days before
`start_date` are left out, the same as everywhere else the balance is counted.
A terminal too short for every bar shows the most recent ones, ending at the
period today falls in, and marks the cut with a `…` in the panel title.

The panel below it is about work alone: one bar per project with its hours and
its share of the range, and `net` — the net time in the range — under them. The
hours are net, each entry carrying its share of its session's break, so they add
up to the `net` on the chart's own footer for the same days. No target, no
balance; those belong to the chart.

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
