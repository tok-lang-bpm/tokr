//! FEEL temporal values of language 0.7 (DMN 1.5 §10.3.2.3): dates, times, dates and
//! times, days-and-time durations and years-and-months durations, with their text forms
//! (XML Schema), arithmetic and comparison.

use std::cmp::Ordering;

const NANOS_PER_SEC: i128 = 1_000_000_000;
const NANOS_PER_DAY: i128 = 86_400 * NANOS_PER_SEC;

/// A calendar date (proleptic Gregorian, years of any sign).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Date {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

/// The time zone of a time or a date and time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Zone {
    /// Local time (no zone).
    None,
    /// An offset from UTC in seconds.
    Offset(i32),
    /// A zone identifier (`@Europe/Warsaw`); `UTC` and `Etc/UTC` have the offset 0.
    Id(String),
}

impl Zone {
    /// The offset in seconds, when it is known.
    pub fn offset(&self) -> Option<i32> {
        match self {
            Zone::None => None,
            Zone::Offset(s) => Some(*s),
            Zone::Id(id) => matches!(
                id.as_str(),
                "UTC" | "Etc/UTC" | "Z" | "GMT" | "Etc/GMT" | "Etc/Zulu" | "Zulu"
            )
            .then_some(0),
        }
    }
    fn has(&self) -> bool {
        !matches!(self, Zone::None)
    }
}

/// A time of day with an optional zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Time {
    /// Nanoseconds since midnight.
    pub nanos: i128,
    pub zone: Zone,
}

/// A date and time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTime {
    pub date: Date,
    pub time: Time,
}

/// A days-and-time duration in nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DtDuration(pub i128);

/// A years-and-months duration in months.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct YmDuration(pub i64);

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

const MAX_YEAR: i64 = 999_999_999;

impl Date {
    pub fn new(year: i64, month: i64, day: i64) -> Option<Date> {
        if !(-MAX_YEAR..=MAX_YEAR).contains(&year) || !(1..=12).contains(&month) {
            return None;
        }
        let (month, day) = (month as u32, u32::try_from(day).ok()?);
        if day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Date { year, month, day })
    }

    /// Days since 1970-01-01.
    pub fn days(&self) -> i64 {
        let y = if self.month <= 2 {
            self.year - 1
        } else {
            self.year
        };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_days(z: i64) -> Option<Date> {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        Date::new(if m <= 2 { y + 1 } else { y }, m, d)
    }

    /// ISO weekday: 1 = Monday … 7 = Sunday.
    pub fn weekday(&self) -> i64 {
        (self.days() + 3).rem_euclid(7) + 1
    }

    pub fn day_of_year(&self) -> i64 {
        self.days() - Date::new(self.year, 1, 1).map(|d| d.days()).unwrap_or(0) + 1
    }

    /// ISO 8601 week number.
    pub fn week_of_year(&self) -> i64 {
        let wd = self.weekday();
        let thursday = self.days() - wd + 4;
        let Some(t) = Date::from_days(thursday) else {
            return 0;
        };
        let jan1 = Date::new(t.year, 1, 1).map(|d| d.days()).unwrap_or(0);
        (thursday - jan1) / 7 + 1
    }

    pub fn add_months(&self, months: i64) -> Option<Date> {
        let total = self.year.checked_mul(12)? + i64::from(self.month) - 1 + months;
        let y = total.div_euclid(12);
        let m = total.rem_euclid(12) + 1;
        let d = i64::from(self.day).min(i64::from(days_in_month(y, m as u32)));
        Date::new(y, m, d)
    }

    pub fn text(&self) -> String {
        let sign = if self.year < 0 { "-" } else { "" };
        format!(
            "{sign}{:04}-{:02}-{:02}",
            self.year.unsigned_abs(),
            self.month,
            self.day
        )
    }

    /// `YYYY-MM-DD` (more year digits and a sign allowed).
    pub fn parse(s: &str) -> Option<Date> {
        let (neg, body) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let mut parts = body.splitn(3, '-');
        let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
        if y.len() < 4
            || (y.len() > 4 && y.starts_with('0'))
            || m.len() != 2
            || d.len() != 2
            || ![y, m, d]
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        let y: i64 = y.parse().ok()?;
        Date::new(if neg { -y } else { y }, m.parse().ok()?, d.parse().ok()?)
    }
}

fn two(s: &str) -> Option<i64> {
    (s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// `+hh:mm`, `-hh:mm`, `+hh:mm:ss` or `Z`.
fn parse_offset(s: &str) -> Option<i32> {
    if s == "Z" || s == "z" {
        return Some(0);
    }
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let parts: Vec<&str> = s[1..].split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let h = two(parts[0])?;
    let m = two(parts[1])?;
    let sec = if parts.len() == 3 { two(parts[2])? } else { 0 };
    if h > 14 || m > 59 || sec > 59 {
        return None;
    }
    Some(sign * (h * 3600 + m * 60 + sec) as i32)
}

fn offset_text(o: i32) -> String {
    if o == 0 {
        return "Z".into();
    }
    let sign = if o < 0 { '-' } else { '+' };
    let a = o.unsigned_abs();
    let (h, m, s) = (a / 3600, (a % 3600) / 60, a % 60);
    if s == 0 {
        format!("{sign}{h:02}:{m:02}")
    } else {
        format!("{sign}{h:02}:{m:02}:{s:02}")
    }
}

fn fraction_text(nanos: i128) -> String {
    if nanos == 0 {
        return String::new();
    }
    let t = format!("{nanos:09}");
    format!(".{}", t.trim_end_matches('0'))
}

impl Time {
    pub fn new(h: i64, m: i64, s: i64, nanos: i128, zone: Zone) -> Option<Time> {
        if !(0..=23).contains(&h) || !(0..=59).contains(&m) || !(0..=59).contains(&s) {
            return None;
        }
        Some(Time {
            nanos: i128::from((h * 60 + m) * 60 + s) * NANOS_PER_SEC + nanos,
            zone,
        })
    }
    pub fn hour(&self) -> i64 {
        (self.nanos / (3600 * NANOS_PER_SEC)) as i64
    }
    pub fn minute(&self) -> i64 {
        ((self.nanos / (60 * NANOS_PER_SEC)) % 60) as i64
    }
    /// Whole seconds.
    pub fn second(&self) -> i64 {
        ((self.nanos / NANOS_PER_SEC) % 60) as i64
    }
    pub fn subsec(&self) -> i128 {
        self.nanos % NANOS_PER_SEC
    }

    /// `hh:mm:ss[.fff][Z|±hh:mm|@zone]`.
    pub fn parse(s: &str) -> Option<Time> {
        // the text before a zone identifier is ASCII (it is read by byte positions)
        if !s.split_once('@').map_or(s, |(b, _)| b).is_ascii() {
            return None;
        }
        let (body, zone) = if let Some((b, z)) = s.split_once('@') {
            if !zone_id(z) {
                return None;
            }
            (b, Zone::Id(z.to_string()))
        } else if let Some(b) = s.strip_suffix('Z') {
            (b, Zone::Offset(0))
        } else if s.len() > 8 && (s[8..].contains('+') || s[8..].contains('-')) {
            let pos = s[8..].find(['+', '-'])? + 8;
            (&s[..pos], Zone::Offset(parse_offset(&s[pos..])?))
        } else {
            (s, Zone::None)
        };
        if body.len() < 8 || body.as_bytes()[2] != b':' || body.as_bytes()[5] != b':' {
            return None;
        }
        let h = two(&body[0..2])?;
        let m = two(&body[3..5])?;
        let sec = two(&body[6..8])?;
        let mut nanos: i128 = 0;
        let rest = &body[8..];
        if !rest.is_empty() {
            let frac = rest.strip_prefix('.')?;
            if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let mut digits: String = frac.chars().take(9).collect();
            while digits.len() < 9 {
                digits.push('0');
            }
            nanos = digits.parse().ok()?;
        }
        if h == 24 && m == 0 && sec == 0 && nanos == 0 {
            return Time::new(0, 0, 0, 0, zone);
        }
        Time::new(h, m, sec, nanos, zone)
    }

    pub fn text(&self) -> String {
        format!(
            "{:02}:{:02}:{:02}{}{}",
            self.hour(),
            self.minute(),
            self.second(),
            fraction_text(self.subsec()),
            zone_text(&self.zone)
        )
    }

    /// Nanoseconds in UTC, when the offset is known; local nanoseconds without a zone.
    fn key(&self) -> (bool, i128) {
        match self.zone.offset() {
            Some(o) => (true, self.nanos - i128::from(o) * NANOS_PER_SEC),
            None => (false, self.nanos),
        }
    }

    pub fn compare(&self, o: &Time) -> Option<Ordering> {
        if (matches!(self.zone, Zone::Id(_)) || matches!(o.zone, Zone::Id(_)))
            && (self.zone.offset().is_none() || o.zone.offset().is_none())
        {
            return (self.zone == o.zone).then(|| self.nanos.cmp(&o.nanos));
        }
        let (a, x) = self.key();
        let (b, y) = o.key();
        if a != b {
            return None;
        }
        Some(
            x.rem_euclid(NANOS_PER_DAY)
                .cmp(&y.rem_euclid(NANOS_PER_DAY)),
        )
    }

    pub fn equals(&self, o: &Time) -> bool {
        // equality has the resolution of a second
        let (a, b) = (
            Time {
                nanos: self.nanos - self.subsec(),
                zone: self.zone.clone(),
            },
            Time {
                nanos: o.nanos - o.subsec(),
                zone: o.zone.clone(),
            },
        );
        a.equals_exact(&b)
    }

    fn equals_exact(&self, o: &Time) -> bool {
        if let (Zone::Id(a), Zone::Id(b)) = (&self.zone, &o.zone)
            && self.zone.offset().is_none()
        {
            return a == b && self.nanos == o.nanos;
        }
        if matches!(self.zone, Zone::Id(_)) != matches!(o.zone, Zone::Id(_))
            && (self.zone.offset().is_none() || o.zone.offset().is_none())
        {
            return false;
        }
        self.compare(o) == Some(Ordering::Equal)
    }

    pub fn add(&self, d: &DtDuration) -> Time {
        Time {
            nanos: (self.nanos + d.0).rem_euclid(NANOS_PER_DAY),
            zone: self.zone.clone(),
        }
    }

    pub fn sub(&self, o: &Time) -> Option<DtDuration> {
        let (a, x) = self.key();
        let (b, y) = o.key();
        if a != b {
            return None;
        }
        Some(DtDuration(x - y))
    }
}

/// A plausible IANA zone identifier: `UTC`, `GMT`, `Z`, or `Area/Place` of a known area.
fn zone_id(z: &str) -> bool {
    const AREAS: &[&str] = &[
        "Africa",
        "America",
        "Antarctica",
        "Arctic",
        "Asia",
        "Atlantic",
        "Australia",
        "Europe",
        "Indian",
        "Pacific",
        "Etc",
        "US",
        "Canada",
        "Brazil",
        "Mexico",
        "Chile",
    ];
    if matches!(z, "UTC" | "GMT" | "Z" | "Zulu") {
        return true;
    }
    let Some((area, place)) = z.split_once('/') else {
        return false;
    };
    AREAS.contains(&area)
        && !place.is_empty()
        && place
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c))
}

fn zone_text(z: &Zone) -> String {
    match z {
        Zone::None => String::new(),
        Zone::Offset(o) => offset_text(*o),
        Zone::Id(id) => format!("@{id}"),
    }
}

impl DateTime {
    /// `date "T" time`, or a date alone (midnight, no zone).
    pub fn parse(s: &str) -> Option<DateTime> {
        match s.find('T') {
            Some(p) => {
                let date = Date::parse(&s[..p])?;
                let time = Time::parse(&s[p + 1..])?;
                // 24:00:00 is the start of the next day
                let date = if s[p + 1..].starts_with("24:") {
                    Date::from_days(date.days() + 1)?
                } else {
                    date
                };
                Some(DateTime { date, time })
            }
            None => Some(DateTime {
                date: Date::parse(s)?,
                time: Time {
                    nanos: 0,
                    zone: Zone::None,
                },
            }),
        }
    }

    pub fn text(&self) -> String {
        format!("{}T{}", self.date.text(), self.time.text())
    }

    pub fn has_zone(&self) -> bool {
        self.time.zone.has()
    }

    /// Local nanoseconds since 1970-01-01T00:00.
    pub fn local(&self) -> i128 {
        i128::from(self.date.days()) * NANOS_PER_DAY + self.time.nanos
    }

    fn key(&self) -> (bool, i128) {
        match self.time.zone.offset() {
            Some(o) => (true, self.local() - i128::from(o) * NANOS_PER_SEC),
            None => (false, self.local()),
        }
    }

    pub fn from_local(local: i128, zone: Zone) -> Option<DateTime> {
        let days = local.div_euclid(NANOS_PER_DAY);
        let nanos = local.rem_euclid(NANOS_PER_DAY);
        Some(DateTime {
            date: Date::from_days(i64::try_from(days).ok()?)?,
            time: Time { nanos, zone },
        })
    }

    pub fn compare(&self, o: &DateTime) -> Option<Ordering> {
        let unknown =
            |d: &DateTime| matches!(d.time.zone, Zone::Id(_)) && d.time.zone.offset().is_none();
        if unknown(self) || unknown(o) {
            return (self.time.zone == o.time.zone).then(|| self.local().cmp(&o.local()));
        }
        let (a, x) = self.key();
        let (b, y) = o.key();
        (a == b).then(|| x.cmp(&y))
    }

    pub fn equals(&self, o: &DateTime) -> bool {
        // equality has the resolution of a second
        let trunc = |d: &DateTime| DateTime {
            date: d.date,
            time: Time {
                nanos: d.time.nanos - d.time.subsec(),
                zone: d.time.zone.clone(),
            },
        };
        trunc(self).equals_exact(&trunc(o))
    }

    fn equals_exact(&self, o: &DateTime) -> bool {
        let unknown =
            |d: &DateTime| matches!(d.time.zone, Zone::Id(_)) && d.time.zone.offset().is_none();
        if unknown(self) || unknown(o) {
            return self.time.zone == o.time.zone && self.local() == o.local();
        }
        self.compare(o) == Some(Ordering::Equal)
    }

    pub fn add(&self, d: &DtDuration) -> Option<DateTime> {
        DateTime::from_local(self.local() + d.0, self.time.zone.clone())
    }

    pub fn add_months(&self, months: i64) -> Option<DateTime> {
        Some(DateTime {
            date: self.date.add_months(months)?,
            time: self.time.clone(),
        })
    }

    pub fn sub(&self, o: &DateTime) -> Option<DtDuration> {
        let unknown =
            |d: &DateTime| matches!(d.time.zone, Zone::Id(_)) && d.time.zone.offset().is_none();
        if unknown(self) || unknown(o) {
            return (self.time.zone == o.time.zone).then(|| DtDuration(self.local() - o.local()));
        }
        let (a, x) = self.key();
        let (b, y) = o.key();
        (a == b).then_some(DtDuration(x - y))
    }

    /// The date as a date and time at midnight UTC (for arithmetic with dates).
    pub fn of_date(d: &Date) -> DateTime {
        DateTime {
            date: *d,
            time: Time {
                nanos: 0,
                zone: Zone::None,
            },
        }
    }
}

impl DtDuration {
    pub fn text(&self) -> String {
        if self.0 == 0 {
            return "PT0S".into();
        }
        let sign = if self.0 < 0 { "-" } else { "" };
        let a = self.0.unsigned_abs() as i128;
        let days = a / NANOS_PER_DAY;
        let rest = a % NANOS_PER_DAY;
        let h = rest / (3600 * NANOS_PER_SEC);
        let m = (rest / (60 * NANOS_PER_SEC)) % 60;
        let s = (rest / NANOS_PER_SEC) % 60;
        let f = rest % NANOS_PER_SEC;
        let mut out = format!("{sign}P");
        if days > 0 {
            out.push_str(&format!("{days}D"));
        }
        if rest > 0 {
            out.push('T');
            if h > 0 {
                out.push_str(&format!("{h}H"));
            }
            if m > 0 {
                out.push_str(&format!("{m}M"));
            }
            if s > 0 || f > 0 {
                out.push_str(&format!("{s}{}S", fraction_text(f)));
            }
        }
        out
    }
    pub fn days(&self) -> i128 {
        self.0 / NANOS_PER_DAY
    }
    pub fn hours(&self) -> i128 {
        (self.0 % NANOS_PER_DAY) / (3600 * NANOS_PER_SEC)
    }
    pub fn minutes(&self) -> i128 {
        (self.0 % (3600 * NANOS_PER_SEC)) / (60 * NANOS_PER_SEC)
    }
    /// Seconds with their fraction, as (whole seconds, nanoseconds), signed.
    pub fn seconds(&self) -> (i128, i128) {
        let r = self.0 % (60 * NANOS_PER_SEC);
        (r / NANOS_PER_SEC, r % NANOS_PER_SEC)
    }
    pub fn nanos(&self) -> i128 {
        self.0
    }
    pub fn from_seconds_text(text: &str) -> Option<DtDuration> {
        // a decimal number of seconds
        let (neg, t) = match text.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, text),
        };
        let (i, f) = t.split_once('.').unwrap_or((t, ""));
        if i.is_empty()
            || !i.bytes().all(|b| b.is_ascii_digit())
            || !f.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let whole: i128 = i.parse().ok()?;
        let mut fd: String = f.chars().take(9).collect();
        while fd.len() < 9 {
            fd.push('0');
        }
        let n = whole.checked_mul(NANOS_PER_SEC)? + fd.parse::<i128>().ok()?;
        Some(DtDuration(if neg { -n } else { n }))
    }
}

impl YmDuration {
    pub fn text(&self) -> String {
        if self.0 == 0 {
            return "P0M".into();
        }
        let sign = if self.0 < 0 { "-" } else { "" };
        let a = self.0.unsigned_abs();
        let (y, m) = (a / 12, a % 12);
        let mut out = format!("{sign}P");
        if y > 0 {
            out.push_str(&format!("{y}Y"));
        }
        if m > 0 {
            out.push_str(&format!("{m}M"));
        }
        out
    }
}

/// A parsed `duration(…)` text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnyDuration {
    Dt(DtDuration),
    Ym(YmDuration),
}

/// XML Schema duration `-?PnYnMnDTnHnMnS`; years and months may not be mixed with days
/// and times (FEEL has no such duration).
pub fn parse_duration(s: &str) -> Option<AnyDuration> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let body = body.strip_prefix('P')?;
    if body.is_empty() {
        return None;
    }
    let (date, time) = match body.split_once('T') {
        Some((d, t)) => {
            if t.is_empty() {
                return None;
            }
            (d, Some(t))
        }
        None => (body, None),
    };
    let mut years: i128 = 0;
    let mut months: i128 = 0;
    let mut days: i128 = 0;
    let mut has_ym = false;
    let mut has_dt = false;
    let mut num = String::new();
    let mut order = 0;
    for c in date.chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        if num.is_empty() {
            return None;
        }
        let v: i128 = num.parse().ok()?;
        num.clear();
        let rank = match c {
            'Y' => {
                years = v;
                has_ym = true;
                1
            }
            'M' => {
                months = v;
                has_ym = true;
                2
            }
            'D' => {
                days = v;
                has_dt = true;
                3
            }
            _ => return None,
        };
        if rank <= order {
            return None;
        }
        order = rank;
    }
    if !num.is_empty() {
        return None;
    }
    let mut nanos: i128 = 0;
    if let Some(t) = time {
        has_dt = true;
        let mut order = 0;
        for c in t.chars() {
            if c.is_ascii_digit() || c == '.' {
                num.push(c);
                continue;
            }
            if num.is_empty() {
                return None;
            }
            let rank = match c {
                'H' => 1,
                'M' => 2,
                'S' => 3,
                _ => return None,
            };
            if rank <= order {
                return None;
            }
            order = rank;
            if c == 'S' {
                nanos += DtDuration::from_seconds_text(&num)?.0;
            } else {
                if num.contains('.') {
                    return None;
                }
                let v: i128 = num.parse().ok()?;
                nanos += v * if c == 'H' { 3600 } else { 60 } * NANOS_PER_SEC;
            }
            num.clear();
        }
        if !num.is_empty() {
            return None;
        }
    }
    if has_ym && has_dt {
        return None;
    }
    let sign = if neg { -1 } else { 1 };
    if has_ym {
        let total = years.checked_mul(12)?.checked_add(months)?;
        return Some(AnyDuration::Ym(YmDuration(
            i64::try_from(total * sign).ok()?,
        )));
    }
    let total = days.checked_mul(NANOS_PER_DAY)?.checked_add(nanos)?;
    Some(AnyDuration::Dt(DtDuration(total * sign)))
}

/// Whole months between two dates, as `years and months duration(from, to)` (the time
/// of day does not count).
pub fn months_between(from: &Date, to: &Date) -> i64 {
    let mut m = (to.year - from.year) * 12 + i64::from(to.month) - i64::from(from.month);
    let later_day = to.day < from.day;
    let earlier_day = to.day > from.day;
    if m > 0 && later_day {
        m -= 1;
    } else if m < 0 && earlier_day {
        m += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts() {
        let d = Date::parse("2027-01-15").unwrap_or(Date {
            year: 0,
            month: 1,
            day: 1,
        });
        assert_eq!(d.text(), "2027-01-15");
        assert_eq!(d.weekday(), 5);
        assert_eq!(Date::from_days(d.days()), Some(d));
        assert_eq!(Date::parse("2027-02-30"), None);
        assert_eq!(
            Time::parse("10:30:00.500+02:00").map(|t| t.text()),
            Some("10:30:00.5+02:00".into())
        );
        assert_eq!(
            parse_duration("P2DT25H").map(|d| match d {
                AnyDuration::Dt(x) => x.text(),
                AnyDuration::Ym(x) => x.text(),
            }),
            Some("P3DT1H".into())
        );
        assert_eq!(
            parse_duration("-P1Y14M").map(|d| match d {
                AnyDuration::Dt(x) => x.text(),
                AnyDuration::Ym(x) => x.text(),
            }),
            Some("-P2Y2M".into())
        );
        assert_eq!(parse_duration("P1Y2D"), None);
        assert_eq!(
            Date::parse("2027-01-31")
                .and_then(|d| d.add_months(1))
                .map(|d| d.text()),
            Some("2027-02-28".into())
        );
    }
}
