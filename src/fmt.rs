//! Number formatting for readouts. Everything here is shown in the mono font.

/// A track time: 0:07, 3:42, 1:02:09.
pub fn time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
}

/// A total length: 42 min, 3 h 12 min.
pub fn total(secs: f64) -> String {
    let m = (secs.max(0.0) / 60.0).round() as u64;
    if m >= 60 { format!("{} h {} min", m / 60, m % 60) } else { format!("{m} min") }
}

/// A runtime: 42 min, 2 h 12 min.
pub fn runtime(secs: f64) -> String {
    total(secs)
}

/// What's left: "23 min left", "1 h 5 min left".
pub fn left(secs: f64) -> String {
    if secs < 60.0 { "Almost done".into() } else { format!("{} left", total(secs)) }
}

/// The clock time `secs` from now: "10:42 PM", or "22:42" where the
/// locale has no AM/PM.
pub fn clock_in(secs: f64) -> String {
    let Ok(now) = gtk::glib::DateTime::now_local() else { return String::new() };
    let Ok(then) = now.add_seconds(secs.max(0.0)) else { return String::new() };
    let twelve = then.format("%p").is_ok_and(|p| !p.is_empty());
    then.format(if twelve { "%-l:%M %p" } else { "%H:%M" }).map(|s| s.to_string()).unwrap_or_default()
}

/// Playback speed: 1, 1.25, 0.5.
pub fn speed(s: f64) -> String {
    let t = format!("{s:.2}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A subtitle delay: 0 s, +0.3 s, -1.2 s.
pub fn delay(secs: f64) -> String {
    if secs.abs() < 0.05 { "0 s".into() } else { format!("{secs:+.1} s") }
}

/// A small offset in milliseconds: "+50 ms", "0 ms".
pub fn delay_ms(secs: f64) -> String {
    let ms = (secs * 1000.0).round() as i64;
    if ms == 0 { "0 ms".into() } else { format!("{ms:+} ms") }
}

/// A file size: 812 MB, 4.2 GB.
pub fn size(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    if b >= 1e9 { format!("{:.1} GB", b / 1e9) } else { format!("{:.0} MB", b / 1e6) }
}

/// "1 video", "12 videos".
pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

/// 1,204
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Decibels for fader readouts: 0, +3.5, -6.0.
pub fn db(v: f64) -> String {
    if v.abs() < 0.05 { "0".into() } else { format!("{v:+.1}") }
}

/// Hz labels: 31, 250, 1k, 16k.
pub fn hz(f: f64) -> String {
    if f >= 1000.0 {
        let k = f / 1000.0;
        if (k - k.round()).abs() < 0.05 { format!("{}k", k.round() as u64) } else { format!("{k:.1}k") }
    } else {
        format!("{}", f.round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times() {
        assert_eq!(time(7.4), "0:07");
        assert_eq!(time(222.0), "3:42");
        assert_eq!(time(3729.0), "1:02:09");
        assert_eq!(total(2520.0), "42 min");
        assert_eq!(total(11520.0), "3 h 12 min");
    }

    #[test]
    fn formats_counts() {
        assert_eq!(count(1, "video", "videos"), "1 video");
        assert_eq!(count(1204, "video", "videos"), "1,204 videos");
        assert_eq!(thousands(123), "123");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn formats_units() {
        assert_eq!(db(0.01), "0");
        assert_eq!(db(3.5), "+3.5");
        assert_eq!(hz(31.0), "31");
        assert_eq!(hz(1000.0), "1k");
        assert_eq!(hz(16000.0), "16k");
        assert_eq!(speed(1.0), "1");
        assert_eq!(speed(1.25), "1.25");
        assert_eq!(speed(0.5), "0.5");
        assert_eq!(delay(0.0), "0 s");
        assert_eq!(delay(-1.2), "-1.2 s");
        assert_eq!(size(4_200_000_000), "4.2 GB");
        assert_eq!(left(1380.0), "23 min left");
    }
}
