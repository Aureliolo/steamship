//! Seconds since 1970 as a day and a time in UTC.

/// The day `seconds` since 1970 fall on, in UTC, as a year, month and day.
#[must_use]
pub fn date(seconds: u64) -> (u64, u64, u64) {
    // Howard Hinnant's civil-from-days: eras of 400 years, each starting on 1 March, so that the
    // leap day falls at the end of a year.
    let days = seconds.div_euclid(86_400).saturating_add(719_468);
    let era = days.div_euclid(146_097);
    let of_era = days.saturating_sub(era.saturating_mul(146_097));
    let year_of_era = of_era
        .saturating_sub(of_era.div_euclid(1_460))
        .saturating_add(of_era.div_euclid(36_524))
        .saturating_sub(of_era.div_euclid(146_096))
        .div_euclid(365);
    let day_of_year = of_era.saturating_sub(
        year_of_era
            .saturating_mul(365)
            .saturating_add(year_of_era.div_euclid(4))
            .saturating_sub(year_of_era.div_euclid(100)),
    );
    let shifted_month = day_of_year
        .saturating_mul(5)
        .saturating_add(2)
        .div_euclid(153);
    let day = day_of_year
        .saturating_sub(
            shifted_month
                .saturating_mul(153)
                .saturating_add(2)
                .div_euclid(5),
        )
        .saturating_add(1);
    let month = if shifted_month < 10 {
        shifted_month.saturating_add(3)
    } else {
        shifted_month.saturating_sub(9)
    };
    let year = year_of_era
        .saturating_add(era.saturating_mul(400))
        .saturating_add(u64::from(month <= 2));
    (year, month, day)
}

/// `seconds` since 1970 as an ISO 8601 time in UTC, such as `2026-10-13T14:02:11Z`.
#[must_use]
pub fn timestamp(seconds: u64) -> String {
    let (year, month, day) = date(seconds);
    let of_day = seconds.rem_euclid(86_400);
    format!(
        "{year}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day.div_euclid(3_600),
        of_day.rem_euclid(3_600).div_euclid(60),
        of_day.rem_euclid(60)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_is_read_off_seconds_since_1970() {
        assert_eq!(date(0), (1970, 1, 1));
        assert_eq!(date(951_782_400), (2000, 2, 29));
        assert_eq!(date(1_790_553_599), (2026, 9, 27));
        assert_eq!(date(4_107_542_400), (2100, 3, 1));
    }

    #[test]
    fn a_time_is_written_in_iso_8601_to_the_second_in_utc() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(timestamp(1_790_553_599), "2026-09-27T23:59:59Z");
        assert_eq!(timestamp(1_791_910_891), "2026-10-13T17:01:31Z");
    }
}
