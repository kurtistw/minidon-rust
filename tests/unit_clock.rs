//! Wave 1 unit tests for `platform::clock` (REQ-011, REQ-012, SC-ACC-018, review M-05, M-08).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use minidon_rust::platform::clock::{
    format_stored, format_wire, parse_stored, Clock, FixedClock, SystemClock,
};
use time::macros::datetime;
use time::{Duration, PrimitiveDateTime};

#[test]
fn fixed_clock_returns_the_instant_it_was_given() {
    let clock = FixedClock::new(datetime!(2026-01-01 00:00:00));
    assert_eq!(clock.now(), datetime!(2026-01-01 00:00:00));
}

#[test]
fn fixed_clock_does_not_move_on_its_own() {
    let clock = FixedClock::new(datetime!(2026-01-01 00:00:00));
    let first = clock.now();
    std::thread::sleep(std::time::Duration::from_millis(5));
    assert_eq!(clock.now(), first);
}

#[test]
fn fixed_clock_can_be_set_and_advanced_to_an_exact_instant() {
    // SC-ACC-018: the expiry boundary needs exactly T0 + 30 days.
    let t0 = datetime!(2026-01-01 00:00:00);
    let clock = FixedClock::new(t0);
    clock.advance(Duration::days(30));
    assert_eq!(clock.now(), datetime!(2026-01-31 00:00:00));
    clock.advance(Duration::seconds(-1));
    assert_eq!(clock.now(), datetime!(2026-01-30 23:59:59));
    clock.set(t0);
    assert_eq!(clock.now(), t0);
}

#[test]
fn fixed_clock_keeps_microseconds_and_drops_finer_precision() {
    let clock = FixedClock::new(datetime!(2026-01-01 00:00:00.123_456_789));
    assert_eq!(clock.now(), datetime!(2026-01-01 00:00:00.123_456));
}

#[test]
fn clock_is_usable_through_a_trait_object_across_threads() {
    let clock: std::sync::Arc<dyn Clock> =
        std::sync::Arc::new(FixedClock::new(datetime!(2026-01-01 00:00:00)));
    let other = clock.clone();
    let handle = std::thread::spawn(move || other.now());
    assert_eq!(handle.join().unwrap(), clock.now());
}

#[test]
fn system_clock_is_naive_utc_with_microsecond_precision() {
    let now = SystemClock.now();
    let reference = time::OffsetDateTime::now_utc();
    let drift = (PrimitiveDateTime::new(reference.date(), reference.time()) - now).abs();
    assert!(drift < Duration::seconds(5), "system clock is off: {drift}");
    assert_eq!(now.nanosecond() % 1000, 0);
}

#[test]
fn stored_format_always_has_six_fraction_digits() {
    assert_eq!(
        format_stored(datetime!(2026-01-01 00:00:00)),
        "2026-01-01 00:00:00.000000"
    );
    assert_eq!(
        format_stored(datetime!(2026-12-31 23:59:59.000_005)),
        "2026-12-31 23:59:59.000005"
    );
}

#[test]
fn stored_text_round_trips() {
    for value in [
        datetime!(2026-01-01 00:00:00),
        datetime!(2026-06-15 12:34:56.789_012),
        datetime!(1999-12-31 23:59:59.999_999),
    ] {
        assert_eq!(parse_stored(&format_stored(value)).unwrap(), value);
    }
}

#[test]
fn parse_stored_accepts_text_without_fraction_and_rejects_garbage() {
    assert_eq!(
        parse_stored("2026-01-01 00:00:00").unwrap(),
        datetime!(2026-01-01 00:00:00)
    );
    for bad in [
        "",
        "2026-01-01T00:00:00.000000",
        "2026-01-01 00:00:00Z",
        "x",
    ] {
        assert!(parse_stored(bad).is_err(), "must reject {bad:?}");
    }
}

#[test]
fn wire_format_is_naive_iso_with_t_and_omits_zero_microseconds() {
    // M-08: confirmed on Python/Pydantic, a zero fraction is not printed.
    assert_eq!(
        format_wire(datetime!(2026-01-01 00:00:00)),
        "2026-01-01T00:00:00"
    );
    assert_eq!(
        format_wire(datetime!(2026-01-01 00:00:00.000_005)),
        "2026-01-01T00:00:00.000005"
    );
    assert_eq!(
        format_wire(datetime!(2026-01-01 10:20:30.5)),
        "2026-01-01T10:20:30.500000"
    );
}
