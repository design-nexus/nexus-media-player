//! Kodi-style `.nfo` files: `<file>.nfo` or `movie.nfo` beside a movie,
//! `<file>.nfo` beside an episode, `tvshow.nfo` in a show's folder. Only the
//! few fields the library shows are read; anything else is ignored.

use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Nfo {
    /// `movie`, `episodedetails` or `tvshow`.
    pub root: String,
    pub title: String,
    pub show: String,
    pub year: Option<i32>,
    pub plot: String,
    pub rating: Option<f64>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub genres: Vec<String>,
    pub tmdb_id: Option<i64>,
}

fn unescape(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")).unwrap_or(s);
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

/// Every `<name …>value</name>` in `text`, in order.
fn all<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut rest = text;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        // `<title>` but not `<titlefoo>`.
        if !after.starts_with('>') && !after.starts_with(' ') {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        if after[..gt].ends_with('/') {
            rest = &after[gt + 1..];
            continue;
        }
        let body = &after[gt + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push(&body[..end]);
        rest = &body[end + close.len()..];
    }
    out
}

fn first(text: &str, name: &str) -> String {
    all(text, name).first().map(|s| unescape(s)).unwrap_or_default()
}

pub fn parse(text: &str) -> Option<Nfo> {
    let root = ["movie", "episodedetails", "tvshow"].into_iter().find(|r| text.contains(&format!("<{r}")))?;
    let year = first(text, "year").parse().ok().or_else(|| {
        let date =
            [first(text, "premiered"), first(text, "aired"), first(text, "releasedate")].into_iter().find(|d| d.len() >= 4)?;
        date[..4].parse().ok()
    });
    // `<rating>8.1</rating>`, or `<ratings><rating default="true"><value>8.1</value>`.
    let rating = first(text, "rating").parse::<f64>().ok().or_else(|| {
        let ratings = all(text, "ratings").first().copied()?;
        first(ratings, "value").parse().ok()
    });
    let tmdb_id = all(text, "uniqueid")
        .into_iter()
        .zip(text.match_indices("<uniqueid").map(|(i, _)| &text[i..]))
        .find(|(_, tag)| tag.split('>').next().is_some_and(|attrs| attrs.contains("tmdb")))
        .and_then(|(v, _)| unescape(v).parse().ok())
        .or_else(|| first(text, "tmdbid").parse().ok());
    Some(Nfo {
        root: root.to_string(),
        title: first(text, "title"),
        show: first(text, "showtitle"),
        year,
        plot: first(text, "plot"),
        rating: rating.filter(|r| *r > 0.0),
        season: first(text, "season").parse().ok(),
        episode: first(text, "episode").parse().ok(),
        genres: all(text, "genre").into_iter().map(unescape).filter(|g| !g.is_empty()).collect(),
        tmdb_id,
    })
}

fn read(path: &Path) -> Option<Nfo> {
    let bytes = std::fs::read(path).ok()?;
    parse(&String::from_utf8_lossy(&bytes))
}

/// The `.nfo` for a video file: `<stem>.nfo`, or `movie.nfo` beside it.
pub fn for_video(video: &Path) -> Option<Nfo> {
    let own = video.with_extension("nfo");
    read(&own).or_else(|| video.parent().and_then(|d| read(&d.join("movie.nfo"))))
}

/// A show's `tvshow.nfo`.
pub fn for_show(folder: &Path) -> Option<Nfo> {
    read(&folder.join("tvshow.nfo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_movies() {
        let n = parse(
            r#"<?xml version="1.0"?><movie><title>Heat &amp; Dust</title><year>1995</year>
            <plot><![CDATA[A crew of thieves.]]></plot>
            <ratings><rating name="imdb" default="true"><value>8.3</value></rating></ratings>
            <genre>Crime</genre><genre>Drama</genre>
            <uniqueid type="imdb">tt0113277</uniqueid><uniqueid type="tmdb" default="true">949</uniqueid></movie>"#,
        )
        .unwrap();
        assert_eq!(n.root, "movie");
        assert_eq!(n.title, "Heat & Dust");
        assert_eq!(n.year, Some(1995));
        assert_eq!(n.plot, "A crew of thieves.");
        assert_eq!(n.rating, Some(8.3));
        assert_eq!(n.genres, vec!["Crime", "Drama"]);
        assert_eq!(n.tmdb_id, Some(949));
    }

    #[test]
    fn reads_episodes_and_shows() {
        let n = parse(
            "<episodedetails><title>Pilot</title><showtitle>Lost</showtitle><season>1</season>\
             <episode>1</episode><aired>2004-09-22</aired><rating>7.9</rating></episodedetails>",
        )
        .unwrap();
        assert_eq!((n.title.as_str(), n.show.as_str(), n.season, n.episode), ("Pilot", "Lost", Some(1), Some(1)));
        assert_eq!(n.year, Some(2004));
        assert_eq!(n.rating, Some(7.9));
        let s = parse("<tvshow><title>Lost</title><premiered>2004-09-22</premiered></tvshow>").unwrap();
        assert_eq!((s.root.as_str(), s.year), ("tvshow", Some(2004)));
        assert!(parse("https://www.imdb.com/title/tt0113277/").is_none());
    }
}
