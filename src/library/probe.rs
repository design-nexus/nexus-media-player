//! What's inside a video file, from `ffprobe`: length, picture size, codecs
//! and the languages of its audio and subtitle tracks.

use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Info {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub vcodec: String,
    pub acodec: String,
    pub audio_langs: Vec<String>,
    pub sub_langs: Vec<String>,
    /// The container's own title tag, if any.
    pub title: String,
}

/// Run a command, giving up (and killing it) after `limit`.
pub fn output_within(cmd: &mut Command, limit: Duration) -> Option<Vec<u8>> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = reader.join().ok()?;
                return status.success().then_some(out);
            }
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// `Ok(None)` when the file has no picture (audio only); `Err` when ffprobe
/// couldn't read it at all.
pub fn probe(path: &Path) -> Result<Option<Info>, ()> {
    let out = output_within(
        Command::new("ffprobe").args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", "--"]).arg(path),
        Duration::from_secs(15),
    )
    .ok_or(())?;
    let v: Value = serde_json::from_slice(&out).map_err(|_| ())?;
    Ok(parse(&v))
}

fn lang(stream: &Value) -> Option<String> {
    let l = stream.pointer("/tags/language")?.as_str()?.trim().to_lowercase();
    (!l.is_empty() && l != "und").then_some(l)
}

pub fn parse(v: &Value) -> Option<Info> {
    let streams = v.get("streams")?.as_array()?;
    let mut info = Info::default();
    let num = |x: Option<&Value>| -> f64 {
        x.and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0.0)
    };
    for s in streams {
        let attached = s.pointer("/disposition/attached_pic").and_then(Value::as_i64) == Some(1);
        match s.get("codec_type").and_then(Value::as_str) {
            Some("video") if !attached && info.vcodec.is_empty() => {
                info.vcodec = s.get("codec_name").and_then(Value::as_str).unwrap_or_default().to_string();
                info.width = num(s.get("width")) as u32;
                info.height = num(s.get("height")) as u32;
            }
            Some("audio") => {
                if info.acodec.is_empty() {
                    info.acodec = s.get("codec_name").and_then(Value::as_str).unwrap_or_default().to_string();
                }
                if let Some(l) = lang(s)
                    && !info.audio_langs.contains(&l)
                {
                    info.audio_langs.push(l);
                }
            }
            Some("subtitle") => {
                if let Some(l) = lang(s)
                    && !info.sub_langs.contains(&l)
                {
                    info.sub_langs.push(l);
                }
            }
            _ => {}
        }
    }
    info.duration = num(v.pointer("/format/duration"));
    if info.duration <= 0.0 {
        info.duration = streams.iter().map(|s| num(s.get("duration"))).fold(0.0, f64::max);
    }
    info.title = v.pointer("/format/tags/title").and_then(Value::as_str).unwrap_or_default().trim().to_string();
    // Audio-only files (or broken ones) aren't videos.
    (!info.vcodec.is_empty()).then_some(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_streams() {
        let v: Value = serde_json::from_str(
            r#"{"streams":[
                {"codec_type":"video","codec_name":"hevc","width":3840,"height":1608},
                {"codec_type":"video","codec_name":"mjpeg","width":600,"height":900,"disposition":{"attached_pic":1}},
                {"codec_type":"audio","codec_name":"eac3","tags":{"language":"eng"}},
                {"codec_type":"audio","codec_name":"aac","tags":{"language":"jpn"}},
                {"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"eng"}},
                {"codec_type":"subtitle","codec_name":"ass","tags":{"language":"und"}}],
              "format":{"duration":"7384.512","tags":{"title":"Dune"}}}"#,
        )
        .unwrap();
        let i = parse(&v).unwrap();
        assert_eq!((i.vcodec.as_str(), i.width, i.height), ("hevc", 3840, 1608));
        assert_eq!(i.acodec, "eac3");
        assert_eq!(i.audio_langs, vec!["eng", "jpn"]);
        assert_eq!(i.sub_langs, vec!["eng"]);
        assert!((i.duration - 7384.512).abs() < 1e-6);
        assert_eq!(i.title, "Dune");
        let audio: Value =
            serde_json::from_str(r#"{"streams":[{"codec_type":"audio","codec_name":"mp3"}],"format":{}}"#).unwrap();
        assert!(parse(&audio).is_none());
    }
}
