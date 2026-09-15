use std::collections::BTreeMap;

use chrono::{Days, NaiveDate};
use rusqlite::{OptionalExtension, params};

use super::{Store, StoreError, StoreResult, date_str, parse_date};
use crate::core::{Day, DayKind, HolidayCalendar, effective_kind};

fn kind_from_row(kind: String, label: Option<String>) -> DayKind {
    DayKind::parse(&kind, label.as_deref()).unwrap_or(DayKind::Work)
}

impl Store {
    pub fn stored_kind(&self, date: NaiveDate) -> StoreResult<Option<DayKind>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT kind, label FROM days WHERE date = ?1",
                [date_str(date)],
                |r| Ok(kind_from_row(r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn set_day_kind(&self, date: NaiveDate, kind: &DayKind) -> StoreResult<()> {
        self.in_write_tx(|| {
            let ds = date_str(date);
            if *kind == DayKind::Work {
                self.conn()
                    .execute("DELETE FROM days WHERE date = ?1", [ds])?;
                return Ok(());
            }
            let n: i64 = self.conn().query_row(
                "SELECT COUNT(*) FROM entries WHERE date = ?1",
                [&ds],
                |r| r.get(0),
            )?;
            if n > 0 {
                return Err(StoreError::Constraint(format!(
                    "{ds} has {n} time entr{}; remove them before changing the day type",
                    if n == 1 { "y" } else { "ies" }
                )));
            }
            self.conn().execute(
                "INSERT INTO days (date, kind, label) VALUES (?1, ?2, ?3)
                 ON CONFLICT(date) DO UPDATE SET kind = excluded.kind, label = excluded.label",
                params![ds, kind.as_str(), kind.label()],
            )?;
            Ok(())
        })
    }

    pub fn stored_kinds_in(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> StoreResult<BTreeMap<NaiveDate, DayKind>> {
        let mut st = self
            .conn()
            .prepare("SELECT date, kind, label FROM days WHERE date BETWEEN ?1 AND ?2")?;
        let rows = st.query_map([date_str(from), date_str(to)], |r| {
            Ok((
                parse_date(&r.get::<_, String>(0)?)?,
                kind_from_row(r.get(1)?, r.get(2)?),
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn days_in(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        cal: &HolidayCalendar,
    ) -> StoreResult<Vec<Day>> {
        let kinds = self.stored_kinds_in(from, to)?;
        let mut by_date: BTreeMap<NaiveDate, Vec<_>> = BTreeMap::new();
        for e in self.entries_in(from, to)? {
            by_date.entry(e.date).or_default().push(e);
        }
        let mut out = Vec::new();
        let mut d = from;
        while d <= to {
            out.push(Day {
                date: d,
                kind: effective_kind(kinds.get(&d), d, cal),
                entries: by_date.remove(&d).unwrap_or_default(),
            });
            d = d.checked_add_days(Days::new(1)).expect("date range");
        }
        Ok(out)
    }
}
