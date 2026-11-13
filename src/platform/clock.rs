//! Time as an input (principle 9). One trait, a system clock and a settable fixed clock.
//!
//! All datetimes are naive UTC with microsecond precision, as Python's `utcnow()` stores them.
//! The fixed clock is for tests and the parity harness only. No production endpoint can reach it.
//!
//! The harness side (review M-05): Python imports `utcnow` by name in `accounts/service.py` and
//! binds it as a column default in `accounts/models.py`, so patching `minidon.db.utcnow` alone
//! does not move sign-in, `authenticate` or `created_at`. The wrapper in the harness copy has to
//! patch every bound reference, or freeze the clock in the process. That is wave 5 work.

use std::sync::atomic::{AtomicI64, Ordering};

use time::format_description::FormatItem;
use time::macros::format_description;
use time::{Duration, OffsetDateTime, PrimitiveDateTime};

/// Naive UTC datetime, the only datetime type the app uses.
pub type NaiveUtc = PrimitiveDateTime;

/// Source of the current time.
pub trait Clock: Send + Sync {
    fn now(&self) -> NaiveUtc;
}

fn to_micros(value: NaiveUtc) -> i64 {
    i64::try_from(value.assume_utc().unix_timestamp_nanos() / 1000).unwrap_or(0)
}

fn from_micros(micros: i64) -> NaiveUtc {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1000)
        .map(|at| PrimitiveDateTime::new(at.date(), at.time()))
        .unwrap_or(PrimitiveDateTime::MIN)
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> NaiveUtc {
        let now = OffsetDateTime::now_utc();
        let micros = i64::try_from(now.unix_timestamp_nanos() / 1000).unwrap_or(0);
        from_micros(micros)
    }
}

/// A clock that stands still until told to move. Lock-free, so a panic cannot poison it.
#[derive(Debug)]
pub struct FixedClock {
    micros: AtomicI64,
}

impl FixedClock {
    pub fn new(at: NaiveUtc) -> Self {
        Self {
            micros: AtomicI64::new(to_micros(at)),
        }
    }

    pub fn set(&self, at: NaiveUtc) {
        self.micros.store(to_micros(at), Ordering::SeqCst);
    }

    pub fn advance(&self, by: Duration) {
        let step = i64::try_from(by.whole_microseconds()).unwrap_or(0);
        self.micros.fetch_add(step, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> NaiveUtc {
        from_micros(self.micros.load(Ordering::SeqCst))
    }
}

const STORED: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:6]");
const STORED_NO_FRACTION: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
const WIRE_BASE: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]");

/// Storage text: `YYYY-MM-DD HH:MM:SS.ffffff`, always six fraction digits (adr-001 D2).
pub fn format_stored(value: NaiveUtc) -> String {
    value.format(STORED).unwrap_or_default()
}

/// Read storage text. Text without a fraction is accepted too, as SQLAlchemy reads it.
pub fn parse_stored(text: &str) -> Result<NaiveUtc, time::error::Parse> {
    PrimitiveDateTime::parse(text, STORED)
        .or_else(|_| PrimitiveDateTime::parse(text, STORED_NO_FRACTION))
}

/// Wire text: naive ISO with `T`. The fraction is left out when the microseconds are zero, as
/// Python and Pydantic do (review M-08, confirmed on Python).
pub fn format_wire(value: NaiveUtc) -> String {
    let base = value.format(WIRE_BASE).unwrap_or_default();
    match value.microsecond() {
        0 => base,
        micro => format!("{base}.{micro:06}"),
    }
}
