//! Names from file and folder names: `Show.Name.S01E02.Title.1080p.mkv`,
//! `Show/Season 1/02 - Title.mkv`, `Movie Title (2019)/Movie.Title.2019.mkv`.

use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parsed {
    pub episode: bool,
    pub title: String,
    pub year: Option<i32>,
    pub show: String,
    pub season: Option<u32>,
    pub number: Option<u32>,
}

static SXXEXX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[ ._\-\[(])s(\d{1,2})[ ._\-]?e(\d{1,3})(?:-?e\d{1,3})*").expect("regex"));
static NXNN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[ ._\-\[(])(\d{1,2})x(\d{2,3})(?:$|[ ._\-\])])").expect("regex"));
static SEASON_DIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:season|series|staffel|saison|temporada)[ ._\-]*(\d{1,3})$|^s(\d{1,2})$").expect("regex")
});
static BARE_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:e|ep|episode)?[ ._\-]*(\d{1,3})(?:$|[ ._\-])").expect("regex"));
static WORD_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[ ._\-])(?:e|ep|episode)[ ._\-]?(\d{1,3})(?:$|[ ._\-])").expect("regex"));
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[ ._\-\[(])((?:19|20)\d{2})").expect("regex"));
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]|\{[^}]*\}").expect("regex"));

/// Words that start the release details, after which nothing is title.
const TAGS: &[&str] = &[
    "2160p",
    "1080p",
    "1080i",
    "720p",
    "576p",
    "480p",
    "4k",
    "uhd",
    "hdr",
    "hdr10",
    "dv",
    "dovi",
    "bluray",
    "blu-ray",
    "bdrip",
    "brrip",
    "bdremux",
    "remux",
    "webrip",
    "web-dl",
    "webdl",
    "web",
    "hdtv",
    "dvdrip",
    "dvd",
    "dvdscr",
    "hdrip",
    "x264",
    "x265",
    "h264",
    "h265",
    "h.264",
    "h.265",
    "hevc",
    "avc",
    "xvid",
    "divx",
    "aac",
    "ac3",
    "eac3",
    "dts",
    "ddp5",
    "dd5",
    "ddp",
    "atmos",
    "truehd",
    "flac",
    "proper",
    "repack",
    "extended",
    "unrated",
    "uncut",
    "directors",
    "imax",
    "10bit",
    "8bit",
    "multi",
    "subbed",
    "dubbed",
    "internal",
    "limited",
    "amzn",
    "nf",
    "dsnp",
    "hmax",
    "atvp",
];

/// Spaces for separators, brackets removed, single spaces.
fn spaced(s: &str) -> String {
    let s = BRACKETS.replace_all(s, " ");
    let s: String = s.chars().map(|c| if c == '.' || c == '_' { ' ' } else { c }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cut a title at the first release tag and tidy its ends.
fn clean(s: &str) -> String {
    let s = spaced(s);
    let mut words = Vec::new();
    for w in s.split(' ') {
        let bare = w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        if TAGS.contains(&bare.as_str()) {
            break;
        }
        words.push(w);
    }
    let out = words.join(" ");
    out.trim_matches(|c: char| c.is_whitespace() || "-–—:,(".contains(c)).trim().to_string()
}

/// A `Season 2` / `S02` folder's number.
pub fn season_folder(name: &str) -> Option<u32> {
    let c = SEASON_DIR.captures(name.trim())?;
    c.get(1).or(c.get(2))?.as_str().parse().ok()
}

/// The year in a name and the title before it: `Heat (1995)` → (Heat, 1995).
fn title_and_year(s: &str) -> (String, Option<i32>) {
    // The last year that isn't the whole name ("1917" alone is a title), and
    // stands alone (not the start of "20491").
    let years: Vec<_> = YEAR
        .captures_iter(s)
        .filter_map(|c| c.get(1))
        .filter(|m| m.start() > 0)
        .filter(|m| s[m.end()..].chars().next().is_none_or(|c| " ._-])".contains(c)))
        .collect();
    match years.last() {
        Some(m) => (clean(&s[..m.start()]), m.as_str().parse().ok()),
        None => (clean(s), None),
    }
}

/// A show's name from its folder (`Show Name (2019)` → `Show Name`).
pub fn show_from_folder(dir: &Path) -> String {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (title, _) = title_and_year(&name);
    if title.is_empty() { spaced(&name) } else { title }
}

pub fn parse(path: &Path) -> Parsed {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let parent = path.parent();
    let parent_name = parent.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let folder_season = season_folder(&parent_name);
    let show_dir = if folder_season.is_some() { parent.and_then(Path::parent) } else { parent };
    let folder_show = || show_dir.map(show_from_folder).unwrap_or_default();

    // Show.S01E02.Title / Show 1x02 Title
    let hit = SXXEXX.captures(&stem).or_else(|| NXNN.captures(&stem));
    if let Some(c) = hit {
        let all = c.get(0).expect("match");
        let season = c.get(1).and_then(|m| m.as_str().parse().ok());
        let number = c.get(2).and_then(|m| m.as_str().parse().ok());
        let before = &stem[..all.start()];
        let (mut show, year) = title_and_year(before);
        if show.is_empty() {
            show = folder_show();
        }
        let title = clean(&stem[all.end()..]);
        return Parsed { episode: true, title, year, show, season, number };
    }

    // Show/Season 1/02 - Title.mkv or Show/Season 1/Show E02.mkv
    if let Some(season) = folder_season {
        let found = BARE_EPISODE.captures(&stem).or_else(|| WORD_EPISODE.captures(&stem));
        if let Some(c) = found {
            let m = c.get(0).expect("match");
            let number = c.get(1).and_then(|m| m.as_str().parse().ok());
            let mut title = clean(&stem[m.end()..]);
            let show = folder_show();
            if title.is_empty() && m.start() > 0 {
                title = clean(&stem[..m.start()]);
            }
            if title.eq_ignore_ascii_case(&show) {
                title.clear();
            }
            return Parsed { episode: true, title, year: None, show, season: Some(season), number };
        }
    }

    // A movie (or something else): the title and year, from the file or its folder.
    let (title, year) = title_and_year(&stem);
    let (folder_title, folder_year) = title_and_year(&parent_name);
    let (title, year) = match (year, folder_year) {
        (Some(y), _) => (if title.is_empty() { folder_title } else { title }, Some(y)),
        // `Heat (1995)/heat.mkv`: the folder knows better.
        (None, Some(y)) if !folder_title.is_empty() => (folder_title, Some(y)),
        _ => (if title.is_empty() { spaced(&stem) } else { title }, None),
    };
    Parsed { episode: false, title, year, show: String::new(), season: None, number: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Parsed {
        parse(Path::new(s))
    }

    #[test]
    fn episodes_from_sxxexx() {
        let e = p("/tv/The.Expanse.S02E05.Home.1080p.WEB-DL.x264.mkv");
        assert!(e.episode);
        assert_eq!((e.show.as_str(), e.season, e.number, e.title.as_str()), ("The Expanse", Some(2), Some(5), "Home"));
        let e = p("/tv/Dark/Season 1/dark s01e03 - past and present.mkv");
        assert_eq!((e.show.as_str(), e.season, e.number, e.title.as_str()), ("dark", Some(1), Some(3), "past and present"));
        let e = p("/tv/Severance/Season 01/S01E01.mkv");
        assert_eq!((e.show.as_str(), e.season, e.number, e.title.as_str()), ("Severance", Some(1), Some(1), ""));
        let e = p("/tv/Doctor.Who.2005.S10E01.The.Pilot.720p.mkv");
        assert_eq!((e.show.as_str(), e.year, e.title.as_str()), ("Doctor Who", Some(2005), "The Pilot"));
        let e = p("/tv/Test Show/Season 2/Test.Show.S02E01.720p.WEB-DL.mkv");
        assert_eq!((e.show.as_str(), e.season, e.number, e.title.as_str()), ("Test Show", Some(2), Some(1), ""));
        let e = p("/tv/Show.S01E01E02.Double.mkv");
        assert_eq!((e.number, e.title.as_str()), (Some(1), "Double"));
        let e = p("/tv/[Group] Show Name - S01E12 [1080p].mkv");
        assert_eq!((e.show.as_str(), e.number), ("Show Name", Some(12)));
    }

    #[test]
    fn episodes_from_nxnn_and_folders() {
        let e = p("/tv/Firefly 1x07 Jaynestown.avi");
        assert_eq!((e.show.as_str(), e.season, e.number, e.title.as_str()), ("Firefly", Some(1), Some(7), "Jaynestown"));
        let e = p("/tv/Better Call Saul (2015)/Season 2/03 - Amarillo.mkv");
        assert_eq!((e.show.as_str(), e.season, e.number, e.title.as_str()), ("Better Call Saul", Some(2), Some(3), "Amarillo"));
        let e = p("/tv/Andor/S01/Andor E04.mkv");
        assert_eq!((e.show.as_str(), e.season, e.number, e.title.as_str()), ("Andor", Some(1), Some(4), ""));
    }

    #[test]
    fn movies() {
        let m = p("/movies/Blade.Runner.2049.2017.2160p.UHD.BluRay.x265.mkv");
        assert!(!m.episode);
        assert_eq!((m.title.as_str(), m.year), ("Blade Runner 2049", Some(2017)));
        let m = p("/movies/Heat (1995)/heat.mkv");
        assert_eq!((m.title.as_str(), m.year), ("Heat", Some(1995)));
        let m = p("/movies/1917.mkv");
        assert_eq!((m.title.as_str(), m.year), ("1917", None));
        let m = p("/movies/Arrival (2016) [1080p].mp4");
        assert_eq!((m.title.as_str(), m.year), ("Arrival", Some(2016)));
        let m = p("/home/Beach_trip_2023.mp4");
        assert_eq!((m.title.as_str(), m.year), ("Beach trip", Some(2023)));
    }

    #[test]
    fn season_folders() {
        assert_eq!(season_folder("Season 3"), Some(3));
        assert_eq!(season_folder("S02"), Some(2));
        assert_eq!(season_folder("season.10"), Some(10));
        assert_eq!(season_folder("Specials"), None);
        assert_eq!(season_folder("Seasonal"), None);
    }
}
