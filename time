#!/usr/bin/env python3

import calendar
import csv
import os
import sys
from collections import namedtuple, defaultdict
from datetime import datetime, timedelta

try:
    from rich.console import Console
    from rich.table import Table
    from rich.panel import Panel
    from rich import box
    from rich.text import Text
    from rich.align import Align
    HAS_RICH = True
except ImportError:
    HAS_RICH = False

# --- Configuration ---
START_DATE_OVERRIDE = "2000-01-01"
INITIAL_BALANCE_HOURS = 00.0
PROJECTS = ["Ausbildung/Studenten", "Black Pearl Change"]
CSV_FILE = os.path.expanduser("~/Timekeeping/timesheet.csv")
SESSION_FILE = os.path.expanduser("~/Timekeeping/.active_in")
DAILY_TARGET = timedelta(hours=1.8)

# --- Data Models ---
# date, project, start_time, end_time, brutto, netto, comment
Entry = namedtuple("Entry", ["date", "project", "start", "end", "brutto", "netto", "comment"])


class Colors:
    HEADER = "\033[95m"
    OKBLUE = "\033[94m"
    OKCYAN = "\033[96m"
    OKGREEN = "\033[92m"
    WARNING = "\033[93m"
    FAIL = "\033[31m"
    ENDC = "\033[0m"
    BOLD = "\033[1m"
    DIM = "\033[90m"
    YELLOW = "\033[33m"


# --- UI and Interaction ---
class UI:
    @staticmethod
    def print_header(text):
        print(f"\n{Colors.BOLD}{Colors.HEADER}--- {text.upper()} ---{Colors.ENDC}")

    @staticmethod
    def print_sub_header(text):
        print(f"{Colors.OKCYAN}{text}{Colors.ENDC}")

    @staticmethod
    def print_error(text):
        print(f"{Colors.FAIL}Error: {text}{Colors.ENDC}")

    @staticmethod
    def print_warning(text):
        print(f"{Colors.WARNING}{text}{Colors.ENDC}")

    @staticmethod
    def print_success(text):
        print(f"{Colors.OKGREEN}{text}{Colors.ENDC}")

    @staticmethod
    def input(prompt):
        user_input = input(prompt)
        if user_input.lower() == "b":
            UI.print_warning("\nOperation cancelled by user. Exiting...")
            sys.exit(0)
        return user_input

    @staticmethod
    def select_from_list(label, items):
        print(f"\n{Colors.BOLD}{label}:{Colors.ENDC}")
        for i, item in enumerate(items):
            print(f"  {Colors.BOLD}{i + 1}){Colors.ENDC} {item}")
        while True:
            try:
                choice = int(UI.input(f"\n{Colors.OKCYAN}?{Colors.ENDC} Choice (1-{len(items)}): "))
                if 1 <= choice <= len(items):
                    return items[choice - 1]
                UI.print_error("Invalid choice.")
            except ValueError:
                UI.print_error("Invalid input. Please enter a number.")


    @staticmethod
    def show_help():
        UI.print_header("Timekeeping Application Help")
        print(f"Usage: {Colors.OKCYAN}./time{Colors.ENDC} {Colors.DIM}[command] [offset/range]{Colors.ENDC}\n")
        
        print(f"{Colors.BOLD}Commands:{Colors.ENDC}")
        print(f"  {Colors.DIM}(no args){Colors.ENDC}             Add entry for today.")
        print(f"  {Colors.OKCYAN}[offset]{Colors.ENDC}              Add entry for a specific day relative to today (e.g., {Colors.DIM}-1{Colors.ENDC} for yesterday).")
        print(f"  {Colors.OKCYAN}--view{Colors.ENDC} {Colors.DIM}[range]{Colors.ENDC}        View the timesheet. {Colors.DIM}range: offset (e.g. 0) or range (e.g. -1 0){Colors.ENDC}.")
        print(f"  {Colors.OKCYAN}--bulk-gleitzeit{Colors.ENDC}      Bulk add flex days (Gleitzeit-Tag).")
        print(f"  {Colors.OKCYAN}--bulk-urlaub{Colors.ENDC}         Bulk add vacation days (Urlaub).")
        print(f"  {Colors.OKCYAN}--in{Colors.ENDC}                  Clock in (starts a session).")
        print(f"  {Colors.OKCYAN}--out{Colors.ENDC}                 Clock out (ends session and records entry).")
        print(f"  {Colors.OKCYAN}--edit{Colors.ENDC} {Colors.DIM}[offset]{Colors.ENDC}       Edit or delete entries for a specific day.")
        print(f"  {Colors.OKCYAN}--help, -h{Colors.ENDC}            Show this help message.")
        
        print(f"\n{Colors.BOLD}Entry Types:{Colors.ENDC}")
        print(f"  Start time: {Colors.OKCYAN}800{Colors.ENDC} {Colors.DIM}(08:00){Colors.ENDC}, {Colors.OKCYAN}0{Colors.ENDC} for {Colors.OKCYAN}Gleitzeit{Colors.ENDC}, {Colors.OKCYAN}u{Colors.ENDC} for {Colors.OKGREEN}Urlaub{Colors.ENDC}.")
        print(f"  End time:   {Colors.OKCYAN}1730{Colors.ENDC} {Colors.DIM}(17:30){Colors.ENDC}.")
        
        print(f"\n{Colors.DIM}Global Exit: Type 'b' at any prompt to exit immediately.{Colors.ENDC}")


# --- Logic and Calculations ---
class Logic:
    @staticmethod
    def parse_time(time_str):
        if not time_str or not time_str.isdigit() or not (3 <= len(time_str) <= 4):
            return None
        h, m = int(time_str[:-2]), int(time_str[-2:])
        return f"{h:02}:{m:02}" if 0 <= h <= 23 and 0 <= m <= 59 else None

    @staticmethod
    def parse_duration(time_str):
        try:
            is_neg = time_str.startswith("-")
            h, m = map(int, time_str.lstrip("+-").split(":"))
            td = timedelta(hours=h, minutes=m)
            return -td if is_neg else td
        except (ValueError, AttributeError):
            return timedelta(0)

    @staticmethod
    def format_duration(td, colored=False):
        is_neg = td.total_seconds() < 0
        total_secs = abs(int(td.total_seconds()))
        h, m = total_secs // 3600, (total_secs % 3600) // 60
        fmt = f"{'-' if is_neg else '+'}{h:02}:{m:02}"
        return f"{Colors.FAIL if is_neg else Colors.OKGREEN}{fmt}{Colors.ENDC}" if colored else fmt

    @staticmethod
    def calculate_brutto(start_str, end_str):
        s = datetime.strptime(start_str, "%H:%M")
        e = datetime.strptime(end_str, "%H:%M")
        if e < s: e += timedelta(days=1)
        return e - s

    @staticmethod
    def calculate_netto(brutto, project):
        if project in ["Gleitzeit-Tag", "Urlaub"]:
            return brutto, timedelta(0)
        deduction = timedelta(minutes=48 if brutto > timedelta(hours=6) else 18 if brutto > timedelta(hours=3) else 0)
        return brutto - deduction, deduction


# --- Data Management ---
class Data:
    @staticmethod
    def load_all():
        entries = []
        earliest = datetime.strptime(START_DATE_OVERRIDE, "%Y-%m-%d").date() if START_DATE_OVERRIDE else None
        if not os.path.exists(CSV_FILE):
            return entries, earliest

        with open(CSV_FILE, "r") as f:
            reader = csv.DictReader(f)
            for r in reader:
                try:
                    dt = datetime.strptime(r["date"], "%Y-%m-%d").date()
                    if earliest is None or dt < earliest: earliest = dt
                    entries.append(Entry(r["date"], r["project"].strip('"'), r["start_time"], r["end_time"], r["brutto"], r["netto"], r["comment"].strip('"')))
                except (ValueError, KeyError): continue
        return entries, earliest

    @staticmethod
    def save_entry(entry):
        header = "date,project,start_time,end_time,brutto,netto,comment\n"
        exists = os.path.exists(CSV_FILE)
        p, c = f'"{entry.project.replace('"', '""')}"', f'"{entry.comment.replace('"', '""')}"'
        line = f"{entry.date},{p},{entry.start},{entry.end},{entry.brutto},{entry.netto},{c}\n"
        with open(CSV_FILE, "a") as f:
            if not exists: f.write(header)
            f.write(line)
        return True

    @staticmethod
    def delete_day(date_str):
        if not os.path.exists(CSV_FILE): return
        with open(CSV_FILE, "r") as f:
            reader = csv.reader(f)
            rows = [row for row in reader if row and row[0] != date_str]
        with open(CSV_FILE, "w") as f:
            writer = csv.writer(f)
            writer.writerows(rows)

    @staticmethod
    def delete_specific(date_str, idx):
        if not os.path.exists(CSV_FILE): return
        with open(CSV_FILE, "r") as f:
            reader = csv.reader(f)
            header = next(reader)
            all_rows = list(reader)
        
        day_rows = sorted([r for r in all_rows if r[0] == date_str], key=lambda x: x[2])
        other_rows = [r for r in all_rows if r[0] != date_str]
        
        if 0 <= idx < len(day_rows):
            rem = day_rows.pop(idx)
            UI.print_success(f"Removed: {rem[2]}-{rem[3]} ({rem[1].strip('\"')})")
        
        with open(CSV_FILE, "w") as f:
            writer = csv.writer(f)
            writer.writerow(header)
            writer.writerows(other_rows + day_rows)


# --- Core Application Logic ---
class TimekeepingApp:
    def __init__(self):
        self.entries, self.earliest_date = Data.load_all()
        self.today = datetime.now().date()

    def get_balances(self):
        gen_bal = timedelta(hours=INITIAL_BALANCE_HOURS)
        mo_bals, wk_bals = {}, {}
        entries_by_date = {}
        for e in self.entries:
            d = datetime.strptime(e.date, "%Y-%m-%d").date()
            entries_by_date.setdefault(d, []).append(e)

        curr = self.earliest_date or self.today
        while curr <= self.today:
            m_key, (y, w, _) = (curr.year, curr.month), curr.isocalendar()
            w_key = (y, w)
            mo_bals.setdefault(m_key, timedelta(0))
            wk_bals.setdefault(w_key, timedelta(0))

            is_urlaub = any(e.project == "Urlaub" for e in entries_by_date.get(curr, []))
            if curr.weekday() < 5 and not is_urlaub:
                gen_bal -= DAILY_TARGET
                mo_bals[m_key] -= DAILY_TARGET
                wk_bals[w_key] -= DAILY_TARGET

            for e in entries_by_date.get(curr, []):
                netto = Logic.parse_duration(e.netto)
                gen_bal += netto
                mo_bals[m_key] += netto
                wk_bals[w_key] += netto
            curr += timedelta(days=1)
        return gen_bal, mo_bals, wk_bals

    def create_entry(self, date_str, force_project=None):
        UI.print_header(f"Recording: {date_str}")
        existing = [e for e in self.entries if e.date == date_str]
        
        while True:
            start_in = UI.input(f"{Colors.OKCYAN}?{Colors.ENDC} Start {Colors.DIM}(e.g. 800, '0' Gleitzeit, 'u' Urlaub){Colors.ENDC}: ").strip().lower()
            if start_in in ["0", "u"]:
                if existing:
                    UI.print_error("Cannot add special day to existing entries.")
                    continue
                proj, s_t, e_t = ("Gleitzeit-Tag" if start_in == "0" else "Urlaub"), "00:00", "00:00"
            else:
                s_t = Logic.parse_time(start_in)
                if not s_t: continue
                if any(e.start == "00:00" and e.end == "00:00" for e in existing):
                    UI.print_error("Day already has Gleitzeit/Urlaub."); continue
                
                e_t = Logic.parse_time(UI.input(f"{Colors.OKCYAN}?{Colors.ENDC} End {Colors.DIM}(e.g. 1730){Colors.ENDC}: "))
                if not e_t: continue
                
                # Overlap check
                ns, ne = datetime.strptime(f"{date_str} {s_t}", "%Y-%m-%d %H:%M"), datetime.strptime(f"{date_str} {e_t}", "%Y-%m-%d %H:%M")
                if ne <= ns: ne += timedelta(days=1)
                overlap = False
                for e in existing:
                    if e.start == "00:00" and e.end == "00:00": continue
                    os, oe = datetime.strptime(f"{e.date} {e.start}", "%Y-%m-%d %H:%M"), datetime.strptime(f"{e.date} {e.end}", "%Y-%m-%d %H:%M")
                    if oe <= os: oe += timedelta(days=1)
                    if ns < oe and ne > os: overlap = True; break
                if overlap: UI.print_error("Overlap detected."); continue
                proj = force_project or UI.select_from_list("Project", PROJECTS)

            comm = proj if proj in ["Gleitzeit-Tag", "Urlaub"] else UI.input(f"{Colors.OKCYAN}?{Colors.ENDC} Comment: ")
            brutto = Logic.calculate_brutto(s_t, e_t)
            netto, ded = Logic.calculate_netto(brutto, proj)
            
            new_e = Entry(date_str, proj, s_t, e_t, Logic.format_duration(brutto), Logic.format_duration(netto), comm)
            if Data.save_entry(new_e):
                UI.print_success(f"Recorded '{proj}' for {date_str}")
                if proj not in ["Gleitzeit-Tag", "Urlaub"]:
                    print(f"Brutto: {new_e.brutto} | Break: {Logic.format_duration(ded)} | Netto: {new_e.netto}")
                break

    def view(self, range_str):
        # Simplification of month parsing
        try:
            offsets = [int(x) for x in range_str.split()]
            if not offsets: offsets = [0]
            elif len(offsets) == 1:
                offsets = [offsets[0], 0] if offsets[0] < 0 else [0, offsets[0]]
            start_off, end_off = min(offsets), max(offsets)
        except ValueError: start_off, end_off = 0, 0

        gen_bal, mo_bals, wk_bals = self.get_balances()
        
        if HAS_RICH:
            console = Console()
            for i in range(start_off, end_off + 1):
                target = self.today + timedelta(days=i * 30) # Rough month offset
                self.render_month_rich(console, target.year, target.month, mo_bals.get((target.year, target.month)), wk_bals)
            
            # General balance footer
            is_neg = gen_bal.total_seconds() < 0
            color = "red" if is_neg else "green"
            bal_text = Logic.format_duration(gen_bal)
            console.print(Align.center(Panel(f"[bold white]GENERAL BALANCE:[/bold white] [bold {color}]{bal_text}[/bold {color}]", box=box.DOUBLE, expand=False), vertical="middle"))
        else:
            for i in range(start_off, end_off + 1):
                target = self.today + timedelta(days=i * 30) # Rough month offset
                self.render_month(target.year, target.month, mo_bals.get((target.year, target.month)), wk_bals)
            
            print("\n" + "=" * 40)
            print(f"GENERAL BALANCE: {Logic.format_duration(gen_bal, colored=True)}")
            print("=" * 40)

    def render_month_rich(self, console, year, month, mo_bal, wk_bals):
        month_name = calendar.month_name[month]
        title = f"[bold cyan]--- {month_name.upper()} {year} ---[/bold cyan]"
        
        table = Table(title=title, box=box.ROUNDED, show_header=True, header_style="bold magenta", expand=True)
        table.add_column("DAY", width=10)
        table.add_column("PROJECT", width=25)
        table.add_column("START", width=8, justify="center")
        table.add_column("END", width=8, justify="center")
        table.add_column("BRUTTO", width=10, justify="right")
        table.add_column("NETTO", width=10, justify="right")
        table.add_column("COMMENT", min_width=20)

        entries_dict = {}
        project_stats = defaultdict(timedelta)
        for e in self.entries:
            dt = datetime.strptime(e.date, "%Y-%m-%d").date()
            if dt.year == year and dt.month == month:
                entries_dict.setdefault(dt.day, []).append(e)
                if e.project not in ["Urlaub", "Gleitzeit-Tag"]:
                    project_stats[e.project] += Logic.parse_duration(e.netto)

        _, last_day = calendar.monthrange(year, month)

        for d in range(1, last_day + 1):
            curr = datetime(year, month, d).date()
            d_str = f"{curr.strftime('%a')} {d:02}"
            
            if d in entries_dict:
                for i, e in enumerate(entries_dict[d]):
                    # Highlight today or past days with entries
                    day_style = "green" if curr <= self.today and i == 0 else "white" if i == 0 else "dim"
                    day_val = Text(d_str if i == 0 else "", style=day_style)
                    
                    if e.project == "Urlaub":
                        table.add_row(day_val, "[bold green]----# URLAUB #----[/bold green]", "", "", "", "", "", style="green")
                    elif e.project == "Gleitzeit-Tag":
                        table.add_row(day_val, "[cyan]Gleitzeit-Tag[/cyan]", "[cyan]X[/cyan]", "[cyan]X[/cyan]", "[cyan]X[/cyan]", "[cyan]X[/cyan]", "[cyan]Gleitzeit-Tag[/cyan]", style="cyan")
                    else:
                        netto_val = e.netto
                        is_neg_netto = "-" in e.netto
                        netto_style = "red" if is_neg_netto else "green"
                        table.add_row(
                            day_val, 
                            e.project, 
                            e.start, 
                            e.end, 
                            e.brutto, 
                            Text(e.netto, style=netto_style), 
                            e.comment
                        )
            else:
                if curr.weekday() >= 5:
                    table.add_row(d_str, "[yellow]----# WEEKEND #----[/yellow]", "", "", "", "", "", style="dim yellow")
                elif curr < self.today:
                    table.add_row(Text(d_str, style="red"), "[bold red]!! NEEDS ATTENTION !![/bold red]", "", "", "", "", "", style="red")
                else:
                    table.add_row(Text(d_str, style="dim"), "[dim]_ [/dim]", "[dim]_ [/dim]", "[dim]_ [/dim]", "[dim]_ [/dim]", "[dim]_ [/dim]", "[dim]_ [/dim]", style="dim")

            if curr.weekday() == 6 or d == last_day:
                y, w, _ = curr.isocalendar()
                bal = wk_bals.get((y, w), timedelta(0))
                is_neg_wk = bal.total_seconds() < 0
                bal_style = "bold red" if is_neg_wk else "bold green"
                bal_fmt = Logic.format_duration(bal)
                
                # Add weekly balance row if it's the end of the week
                if curr.weekday() == 6:
                    table.add_section()
                    table.add_row("", f"[dim italic]WEEKLY BALANCE (KW {w})[/dim italic]", "", "", "", Text(bal_fmt, style=bal_style), "")
                    table.add_section()

        console.print(table)
        
        # Monthly Project Breakdown
        if project_stats:
            stats_table = Table(title=f"[bold cyan]Project Summary - {month_name}[/bold cyan]", box=box.SIMPLE_HEAD, show_header=True, header_style="bold blue", expand=False)
            stats_table.add_column("Project", style="cyan")
            stats_table.add_column("Total Time", justify="right", style="green")
            
            for proj, time_sum in sorted(project_stats.items()):
                stats_table.add_row(proj, Logic.format_duration(time_sum))
            
            # Monthly balance footer
            if mo_bal:
                is_neg_mo = mo_bal.total_seconds() < 0
                mo_style = "bold red" if is_neg_mo else "bold green"
                stats_table.add_section()
                stats_table.add_row("[bold]TOTAL MONTHLY BALANCE[/bold]", Text(Logic.format_duration(mo_bal), style=mo_style))
            
            console.print(Align.right(stats_table))
            console.print("\n")

    def render_month(self, year, month, mo_bal, wk_bals):
        UI.print_sub_header(f"\n--- {calendar.month_name[month].upper()} {year} ---")
        header = ["DAY", "PROJECT", "START", "END", "BRUTTO", "NETTO", "COMMENT"]
        col_w = [8, 20, 7, 7, 8, 8, 25]
        print("  ".join(h.ljust(col_w[i]) for i, h in enumerate(header)))
        print("  ".join("-" * col_w[i] for i in range(len(header))))

        entries_dict = {}
        for e in self.entries:
            dt = datetime.strptime(e.date, "%Y-%m-%d")
            if dt.year == year and dt.month == month: entries_dict.setdefault(dt.day, []).append(e)

        _, last_day = calendar.monthrange(year, month)
        total_width = sum(col_w) + 12

        for d in range(1, last_day + 1):
            curr = datetime(year, month, d)
            d_str = f"{curr.strftime('%a')} {d:02}"
            
            if d in entries_dict:
                for i, e in enumerate(entries_dict[d]):
                    day_col = f"{Colors.OKGREEN}{d_str}{Colors.ENDC}" if i == 0 and curr.date() <= self.today else d_str if i == 0 else ""
                    if e.project == "Urlaub":
                        print(f"{Colors.OKGREEN}{'----# URLAUB #----'.center(total_width)}{Colors.ENDC}")
                    elif e.project == "Gleitzeit-Tag":
                        print(f"  ".join([day_col.ljust(col_w[0] + (10 if i==0 else 0)), "X".center(col_w[1]), "X".center(col_w[2]), "X".center(col_w[3]), "X".center(col_w[4]), "X".center(col_w[5]), "Gleitzeit-Tag"]))
                    else:
                        print("  ".join([day_col.ljust(col_w[0] + (10 if i==0 else 0)), e.project[:col_w[1]].ljust(col_w[1]), e.start.ljust(col_w[2]), e.end.ljust(col_w[3]), e.brutto.ljust(col_w[4]), e.netto.ljust(col_w[5]), e.comment[:col_w[6]]]))
            else:
                if curr.weekday() >= 5: print(f"{Colors.YELLOW}{'----# WEEKEND #----'.center(total_width)}{Colors.ENDC}")
                elif curr.date() < self.today: print(f"{Colors.FAIL}{d_str.ljust(col_w[0])} {'!! NEEDS ATTENTION !!'.center(total_width-col_w[0])}{Colors.ENDC}")
                else: print(f"{Colors.DIM}{d_str.ljust(col_w[0])} {'_'.center(col_w[1])} {'_'.center(col_w[2])} {'_'.center(col_w[3])} {'_'.center(col_w[4])} {'_'.center(col_w[5])} {'_'.center(col_w[6])}{Colors.ENDC}")

            if curr.weekday() == 6:
                y, w, _ = curr.isocalendar()
                bal = wk_bals.get((y, w), timedelta(0))
                lbl = f" WEEKLY BALANCE (KW {w}): "
                print(f"{' '*(total_width-len(lbl)-6)}{Colors.DIM}{lbl}{Colors.ENDC}{Logic.format_duration(bal, True)}")
                print("  ".join("-" * col_w[i] for i in range(len(header))))

        if mo_bal:
            lbl = "MONTHLY BALANCE: "
            print(f"{' '*(total_width-len(lbl)-6)}{lbl}{Logic.format_duration(mo_bal, True)}")

    def edit(self, offset):
        target = self.today + timedelta(days=offset)
        d_str = target.strftime("%Y-%m-%d")
        while True:
            UI.print_header(f"Editing: {d_str}")
            day_entries = sorted([e for e in self.entries if e.date == d_str], key=lambda x: x.start)
            if not day_entries:
                if UI.input("No entries. Add one? (y/n): ").lower() == "y": self.create_entry(d_str); self.entries, _ = Data.load_all()
                else: break
                continue
            
            for i, e in enumerate(day_entries):
                print(f"  {Colors.BOLD}{i+1}){Colors.ENDC} {e.start}-{e.end} ({e.project})")
            
            choice = UI.input("\n(a)dd, (d)elete, (c)lear, (b)ack: ").lower()
            if choice == "a": self.create_entry(d_str); self.entries, _ = Data.load_all()
            elif choice == "d":
                try:
                    idx = int(UI.input("Index: ")) - 1
                    Data.delete_specific(d_str, idx); self.entries, _ = Data.load_all()
                except ValueError: pass
            elif choice == "c": Data.delete_day(d_str); self.entries, _ = Data.load_all()
            elif choice == "b": break

    def clock_in(self):
        if self.today.weekday() >= 5: return UI.print_error("Cannot clock in on weekends.")
        if os.path.exists(SESSION_FILE):
            if UI.input("Already clocked in. Overwrite? (y/n): ").lower() != "y": return
        with open(SESSION_FILE, "w") as f: f.write(f"{self.today} {datetime.now().strftime('%H:%M')}")
        UI.print_success(f"Clocked in at {datetime.now().strftime('%H:%M')}")

    def clock_out(self):
        if not os.path.exists(SESSION_FILE): return UI.print_error("Not clocked in.")
        with open(SESSION_FILE, "r") as f: d_str, s_t = f.read().split()
        UI.print_header(f"Clock Out | Start: {s_t}")
        self.create_entry(d_str)
        os.remove(SESSION_FILE)


def main():
    app = TimekeepingApp()
    args = sys.argv[1:]
    if not args: app.create_entry(app.today.strftime("%Y-%m-%d"))
    elif args[0] in ["--help", "-h"]: UI.show_help()
    elif args[0] == "--view": app.view(args[1] if len(args) > 1 else "0")
    elif args[0] == "--in": app.clock_in()
    elif args[0] == "--out": app.clock_out()
    elif args[0] == "--edit": app.edit(int(args[1]) if len(args) > 1 else 0)
    elif args[0] in ["--bulk-gleitzeit", "--bulk-urlaub"]:
        UI.print_header("Bulk Entry")
        start = UI.input("Start (YYYY-MM-DD or offset): ")
        end = UI.input("End (YYYY-MM-DD or offset): ")
        # Simple bulk implementation
        s_dt = app.today + timedelta(days=int(start)) if start.replace("-","").isdigit() else datetime.strptime(start, "%Y-%m-%d").date()
        e_dt = app.today + timedelta(days=int(end)) if end.replace("-","").isdigit() else datetime.strptime(end, "%Y-%m-%d").date()
        curr, count = s_dt, 0
        while curr <= e_dt:
            if curr.weekday() < 5 and not any(e.date == str(curr) for e in app.entries):
                proj = "Gleitzeit-Tag" if args[0] == "--bulk-gleitzeit" else "Urlaub"
                Data.save_entry(Entry(str(curr), proj, "00:00", "00:00", "00:00", "00:00", proj))
                count += 1
            curr += timedelta(days=1)
        UI.print_success(f"Recorded {count} days.")
    else:
        try: app.create_entry((app.today + timedelta(days=int(args[0]))).strftime("%Y-%m-%d"))
        except ValueError: UI.print_error("Unknown command.")

if __name__ == "__main__":
    main()
