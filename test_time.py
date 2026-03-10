import os
import csv
import pytest
from datetime import datetime, timedelta
from unittest.mock import patch, MagicMock
from timekeeping import Logic, Data, Entry, TimekeepingApp, UI

# Note: I'll assume the script is imported as timekeeping
# In the actual environment, I might need to rename time.py to timekeeping.py 
# or use a hack to import it because 'time' is a standard library name.

def test_parse_time():
    assert Logic.parse_time("800") == "08:00"
    assert Logic.parse_time("1730") == "17:30"
    assert Logic.parse_time("9") is None
    assert Logic.parse_time("2500") is None
    assert Logic.parse_time("abc") is None

def test_parse_duration():
    assert Logic.parse_duration("+01:30") == timedelta(hours=1, minutes=30)
    assert Logic.parse_duration("-00:45") == timedelta(hours=0, minutes=-45)
    assert Logic.parse_duration("invalid") == timedelta(0)

def test_format_duration():
    assert Logic.format_duration(timedelta(hours=1, minutes=30)) == "+01:30"
    assert Logic.format_duration(timedelta(hours=-1, minutes=-30)) == "-01:30"
    # Testing colored output
    formatted = Logic.format_duration(timedelta(hours=1), colored=True)
    assert "\033[92m" in formatted # OKGREEN

def test_calculate_brutto():
    assert Logic.calculate_brutto("08:00", "10:00") == timedelta(hours=2)
    # Overnight
    assert Logic.calculate_brutto("22:00", "02:00") == timedelta(hours=4)

def test_calculate_netto():
    # No break (<3h)
    brutto = timedelta(hours=2)
    netto, ded = Logic.calculate_netto(brutto, "Project")
    assert netto == brutto
    assert ded == timedelta(0)

    # 18 min break (>3h)
    brutto = timedelta(hours=4)
    netto, ded = Logic.calculate_netto(brutto, "Project")
    assert ded == timedelta(minutes=18)
    assert netto == brutto - ded

    # 48 min break (>6h)
    brutto = timedelta(hours=7)
    netto, ded = Logic.calculate_netto(brutto, "Project")
    assert ded == timedelta(minutes=48)
    assert netto == brutto - ded

    # Special projects
    brutto = timedelta(hours=8)
    netto, ded = Logic.calculate_netto(brutto, "Urlaub")
    assert ded == timedelta(0)
    assert netto == brutto

@patch("timekeeping.CSV_FILE", "test_timesheet.csv")
@patch("timekeeping.START_DATE_OVERRIDE", None)
def test_data_save_and_load(tmp_path):
    test_csv = tmp_path / "test_timesheet.csv"
    with patch("timekeeping.CSV_FILE", str(test_csv)):
        entry = Entry("2026-03-01", "Test", "08:00", "10:00", "+02:00", "+02:00", "Comment")
        Data.save_entry(entry)
        
        entries, earliest = Data.load_all()
        assert len(entries) == 1
        assert entries[0].project == "Test"
        assert earliest == datetime.strptime("2026-03-01", "%Y-%m-%d").date()

@patch("timekeeping.CSV_FILE", "test_timesheet.csv")
def test_data_delete(tmp_path):
    test_csv = tmp_path / "test_timesheet.csv"
    with patch("timekeeping.CSV_FILE", str(test_csv)):
        entry1 = Entry("2026-03-01", "P1", "08:00", "09:00", "+01:00", "+01:00", "")
        entry2 = Entry("2026-03-02", "P2", "08:00", "09:00", "+01:00", "+01:00", "")
        Data.save_entry(entry1)
        Data.save_entry(entry2)
        
        Data.delete_day("2026-03-01")
        entries, _ = Data.load_all()
        assert len(entries) == 1
        assert entries[0].date == "2026-03-02"

@patch("timekeeping.CSV_FILE", "test_timesheet.csv")
@patch("timekeeping.START_DATE_OVERRIDE", None)
def test_get_balances(tmp_path):
    test_csv = tmp_path / "test_timesheet.csv"
    with patch("timekeeping.CSV_FILE", str(test_csv)):
        # Mon Mar 2 2026 (Workday)
        # We work 2 hours, target is 1.8h. Balance should be +0.2h (12 min)
        entry = Entry("2026-03-02", "Project", "08:00", "10:00", "+02:00", "+02:00", "")
        Data.save_entry(entry)
        
        # Mock today to Mar 2 2026
        with patch("timekeeping.datetime") as mock_date:
            mock_date.now.return_value = datetime(2026, 3, 2, 12, 0)
            mock_date.strptime = datetime.strptime
            
            app = TimekeepingApp()
            # earliest_date will be picked from CSV since START_DATE_OVERRIDE is None
            gen, mo, wk = app.get_balances()
            assert gen == timedelta(minutes=12)
            assert wk[(2026, 10)] == timedelta(minutes=12)

@patch("timekeeping.UI.input")
@patch("timekeeping.Data.save_entry")
def test_create_entry(mock_save, mock_input):
    mock_input.side_effect = ["800", "1000", "1", "My comment"] # Start, End, Project Choice, Comment
    app = TimekeepingApp()
    app.entries = []
    app.create_entry("2026-03-02")
    
    assert mock_save.called
    saved_entry = mock_save.call_args[0][0]
    assert saved_entry.start == "08:00"
    assert saved_entry.end == "10:00"
    assert saved_entry.comment == "My comment"

def test_ui_methods(capsys):
    UI.print_header("test")
    assert "--- TEST ---" in capsys.readouterr().out
    UI.print_error("err")
    assert "Error: err" in capsys.readouterr().out

@patch("timekeeping.UI.input")
def test_ui_select_from_list(mock_input):
    mock_input.side_effect = ["invalid", "2"]
    choice = UI.select_from_list("Title", ["A", "B"])
    assert choice == "B"

def test_logic_overnight():
    assert Logic.calculate_brutto("23:00", "01:00") == timedelta(hours=2)

@patch("timekeeping.CSV_FILE", "test_timesheet.csv")
@patch("timekeeping.START_DATE_OVERRIDE", None)
def test_balances_with_urlaub(tmp_path):
    test_csv = tmp_path / "test_timesheet.csv"
    with patch("timekeeping.CSV_FILE", str(test_csv)):
        # Mon Mar 2 2026 is Urlaub. Target 1.8h should NOT be deducted.
        entry = Entry("2026-03-02", "Urlaub", "00:00", "00:00", "+00:00", "+00:00", "Vacation")
        Data.save_entry(entry)
        
        with patch("timekeeping.datetime") as mock_date:
            mock_date.now.return_value = datetime(2026, 3, 2, 12, 0)
            mock_date.strptime = datetime.strptime
            app = TimekeepingApp()
            gen, _, _ = app.get_balances()
            assert gen == timedelta(0) # No work, but no deduction either

@patch("timekeeping.UI.input")
@patch("timekeeping.Data.save_entry")
def test_create_entry_gleitzeit(mock_save, mock_input):
    mock_input.side_effect = ["0"] # Choice '0' for Gleitzeit
    app = TimekeepingApp()
    app.entries = []
    app.create_entry("2026-03-02")
    
    saved = mock_save.call_args[0][0]
    assert saved.project == "Gleitzeit-Tag"
    assert saved.netto == "+00:00"

@patch("timekeeping.UI.input")
def test_edit_day_delete(mock_input, tmp_path):
    test_csv = tmp_path / "test_timesheet.csv"
    with patch("timekeeping.CSV_FILE", str(test_csv)):
        entry = Entry("2026-03-02", "P1", "08:00", "09:00", "+01:00", "+01:00", "")
        Data.save_entry(entry)
        
        # Mock today to 2026-03-02
        with patch("timekeeping.datetime") as mock_date:
            mock_date.now.return_value = datetime(2026, 3, 2, 12, 0)
            mock_date.strptime = datetime.strptime
            
            app = TimekeepingApp()
            # Mock input sequence: 'd' (delete), '1' (index), 'b' (back)
            mock_input.side_effect = ["d", "1", "b"]
            app.edit(0) # Edit today (which is mocked to 03-02)
            
            entries, _ = Data.load_all()
            assert len(entries) == 0

@patch("timekeeping.sys.argv", ["time", "--view", "0"])
@patch("timekeeping.UI.input")
def test_main_view(mock_input):
    # This just ensures main doesn't crash when called with --view
    with patch("timekeeping.TimekeepingApp.render_month") as mock_render:
        import timekeeping
        timekeeping.main()
        assert mock_render.called

@patch("timekeeping.SESSION_FILE", "test_session.in")
@patch("timekeeping.UI.input")
def test_clock_in_overwrite(mock_input, tmp_path):
    test_session = tmp_path / "test_session.in"
    with patch("timekeeping.SESSION_FILE", str(test_session)):
        # Create existing session
        test_session.write_text("2026-03-02 07:00")
        
        with patch("timekeeping.datetime") as mock_date:
            mock_date.now.return_value = datetime(2026, 3, 2, 8, 0)
            mock_date.strptime = datetime.strptime
            
            app = TimekeepingApp()
            mock_input.return_value = "y" # Overwrite
            app.clock_in()
            
            assert "08:00" in test_session.read_text()

@patch("timekeeping.Data.save_entry")
@patch("timekeeping.UI.input")
@patch("timekeeping.CSV_FILE", "non_existent.csv")
def test_bulk_entry(mock_input, mock_save):
    # Mock --bulk-gleitzeit
    with patch("timekeeping.sys.argv", ["time", "--bulk-gleitzeit"]):
        # Mon Mar 2 2026 to Tue Mar 3 2026 (Offsets relative to Fri Mar 6)
        # Offset -4 is Mon, -3 is Tue.
        mock_input.side_effect = ["-4", "-3"]
        import timekeeping
        timekeeping.main()
        # Should call save_entry for Mon and Tue
        assert mock_save.call_count == 2

