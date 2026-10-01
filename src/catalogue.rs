use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashSet;

pub const SITE: &str = "https://www.reframed.gallery";
#[derive(Clone, Debug, Deserialize)]
pub struct Artwork {
    pub id: String,
    #[serde(rename = "r2Key")]
    pub key: String,
    pub alt: String,
    pub href: String,
}
impl Artwork {
    pub fn artist(&self) -> &str {
        self.key
            .strip_prefix("originals/")
            .unwrap_or(&self.key)
            .split(" - ")
            .next()
            .unwrap_or("Unknown artist")
    }
    pub fn title(&self) -> &str {
        &self.alt
    }
    pub fn page_url(&self) -> Result<String> {
        let u = url::Url::parse(SITE)?.join(&self.href)?;
        if u.scheme() != "https"
            || u.host_str() != Some("www.reframed.gallery")
            || !u.username().is_empty()
            || u.password().is_some()
            || u.port().is_some()
        {
            bail!("Invalid artwork link");
        }
        Ok(u.to_string())
    }
    pub fn original_url(&self) -> String {
        cdn_url(&self.key, false)
    }
    pub fn preview_url(&self) -> String {
        cdn_url(&self.key, true)
    }
    pub fn hero_preview_url(&self) -> String {
        transformed_url(&self.key, Some("width=1400,quality=85,format=auto"))
    }
}
pub fn cdn_url(key: &str, preview: bool) -> String {
    transformed_url(key, preview.then_some("width=700,quality=80,format=auto"))
}
fn transformed_url(key: &str, settings: Option<&str>) -> String {
    let mut u = url::Url::parse("https://cdn.reframed.gallery/").unwrap();
    {
        let mut segments = u.path_segments_mut().unwrap();
        if let Some(settings) = settings {
            segments.push("cdn-cgi").push("image").push(settings);
        }
        for segment in key.split('/') {
            segments.push(segment);
        }
    }
    u.to_string()
}
pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(40))
        .build()
}
pub fn get_text(url: &str) -> Result<String> {
    let response = agent()
        .get(url)
        .set("User-Agent", "Mozilla/5.0 (Reframed desktop)")
        .set("Referer", SITE)
        .call()?;
    let mut reader = response.into_reader().take(8 * 1024 * 1024 + 1);
    let mut body = String::new();
    std::io::Read::read_to_string(&mut reader, &mut body)?;
    if body.len() > 8 * 1024 * 1024 {
        bail!("Catalogue response too large");
    }
    Ok(body)
}
use std::io::Read;
pub fn fetch_page(page: usize) -> Result<Vec<Artwork>> {
    let path = if page == 1 {
        format!("{SITE}/recent")
    } else {
        format!("{SITE}/recent/page/{page}")
    };
    match get_text(&path) {
        Ok(html) => parse_catalogue(&html),
        Err(error) => {
            if let Some(ureq::Error::Status(status, _)) = error.downcast_ref::<ureq::Error>()
                && is_pagination_end(page, *status)
            {
                return Ok(Vec::new());
            }
            Err(error)
        }
    }
}
/// The site's navigation search returns its top matches without pagination.
pub fn search_url(query: &str) -> Result<url::Url> {
    let mut url = url::Url::parse(SITE)?.join("/api/search/nav")?;
    url.query_pairs_mut().append_pair("q", query.trim());
    Ok(url)
}
pub fn fetch_search(query: &str) -> Result<Vec<Artwork>> {
    if query.trim().chars().count() < 2 {
        return Ok(Vec::new());
    }
    parse_search(&get_text(search_url(query)?.as_str())?)
}
pub fn parse_search(body: &str) -> Result<Vec<Artwork>> {
    #[derive(Deserialize)]
    struct Response {
        results: Vec<serde_json::Value>,
    }
    #[derive(Deserialize)]
    struct Hit {
        id: String,
        title: String,
        r2_key: String,
        href: String,
    }
    let response: Response = serde_json::from_str(body).context("Invalid search response")?;
    let mut artworks = Vec::new();
    let mut seen = HashSet::new();
    for result in response.results {
        let kind = result
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .context("Search result is missing its kind")?;
        if kind != "artwork" {
            continue;
        }
        let hit: Hit = serde_json::from_value(result).context("Invalid artwork search result")?;
        let art = Artwork {
            id: hit.id,
            key: hit.r2_key,
            alt: hit.title,
            href: hit.href,
        };
        if !valid_artwork(&art) {
            bail!("Unsafe or incomplete artwork search metadata");
        }
        if seen.insert(art.id.clone()) {
            artworks.push(art);
        }
    }
    Ok(artworks)
}
fn valid_artwork(item: &Artwork) -> bool {
    !item.id.is_empty()
        && !item.alt.is_empty()
        && item.key.starts_with("originals/")
        && !item
            .key
            .split('/')
            .any(|s| s == "." || s == ".." || s.is_empty())
        && item.page_url().is_ok()
}
fn is_pagination_end(page: usize, status: u16) -> bool {
    page > 1 && status == 404
}
pub fn parse_catalogue(html: &str) -> Result<Vec<Artwork>> {
    let marker = "self.__next_f.push([1,";
    let mut flight = String::new();
    for chunk in html.split(marker).skip(1) {
        let mut de = serde_json::Deserializer::from_str(chunk).into_iter::<String>();
        if let Some(Ok(s)) = de.next() {
            flight.push_str(&s);
        }
    }
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut empty_grid = false;
    // Deserialize from each opening brace: serde reads exactly one balanced object,
    // including braces and escaped quotes inside strings.
    for (offset, _) in flight.match_indices('{') {
        let mut props =
            serde_json::Deserializer::from_str(&flight[offset..]).into_iter::<serde_json::Value>();
        if let Some(Ok(props)) = props.next()
            && props["className"].as_str().is_some_and(|name| {
                name.starts_with("TileGrid-module__") && name.ends_with("__grid")
            })
            && props["children"].as_array().is_some_and(Vec::is_empty)
        {
            empty_grid = true;
        }
        let mut de = serde_json::Deserializer::from_str(&flight[offset..]).into_iter::<Artwork>();
        if let Some(Ok(item)) = de.next()
            && valid_artwork(&item)
            && seen.insert(item.id.clone())
        {
            found.push(item);
        }
    }
    if found.is_empty() && !empty_grid {
        bail!("No artwork metadata found. Reframed may have changed its catalogue format.");
    }
    Ok(found)
}
#[derive(Clone, Debug)]
pub struct Detail {
    pub title: String,
    pub artist: String,
    pub original: String,
    pub dimensions: Option<String>,
}
pub fn fetch_detail(art: &Artwork) -> Result<Detail> {
    let html = get_text(&art.page_url()?)?;
    for chunk in html.split("<script").skip(1) {
        let Some((tag, rest)) = chunk.split_once('>') else {
            continue;
        };
        if !tag.contains("application/ld+json") {
            continue;
        }
        let Some((json, _)) = rest.split_once("</script>") else {
            continue;
        };
        let v: serde_json::Value =
            serde_json::from_str(json).context("Invalid artwork metadata")?;
        if v["@type"] != "VisualArtwork" {
            continue;
        }
        let original = v["contentUrl"]
            .as_str()
            .context("Missing original URL")?
            .to_string();
        crate::download::validate_url(&original)?;
        let dimensions = v["width"]["name"]
            .as_str()
            .zip(v["height"]["name"].as_str())
            .map(|(w, h)| format!("{w} × {h}"));
        return Ok(Detail {
            title: v["name"].as_str().unwrap_or(&art.alt).into(),
            artist: v["artist"]["name"].as_str().unwrap_or(art.artist()).into(),
            original,
            dimensions,
        });
    }
    bail!("Original artwork metadata unavailable. Please retry or view on Reframed.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_maps_only_artworks_and_preserves_server_matches() {
        let body = r#"{"results":[{"kind":"artist","id":"artist"},{"kind":"tag","id":"tag"},{"kind":"artwork","id":"one","r2_key":"originals/Édouard Manet - Café.jpg","title":"Café","href":"/artwork/one"}]}"#;
        let hits = parse_search(body).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title(), "Café");
        assert_eq!(hits[0].artist(), "Édouard Manet");
        assert!(parse_search(r#"{"results":[]}"#).unwrap().is_empty());
        assert!(
            parse_search(r#"{"results":[{"kind":"collection"}]}"#)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn search_rejects_malformed_and_unsafe_metadata() {
        for body in [
            "{}",
            r#"{"results":null}"#,
            r#"{"results":[{}]}"#,
            r#"{"results":[{"kind":"artwork"}]}"#,
        ] {
            assert!(parse_search(body).is_err());
        }
        for (key, href) in [
            ("originals/../evil.jpg", "/artwork/one"),
            ("other/a.jpg", "/artwork/one"),
            ("originals/a.jpg", "https://evil.example/art"),
        ] {
            let body = serde_json::json!({"results":[{"kind":"artwork","id":"one","title":"Title","r2_key":key,"href":href}]}).to_string();
            assert!(parse_search(&body).is_err());
        }
    }
    #[test]
    fn search_url_encodes_trimmed_unicode_and_reserved_characters() {
        let query = " Édouard & café + #? ";
        let url = search_url(query).unwrap();
        assert_eq!(url.host_str(), Some("www.reframed.gallery"));
        assert_eq!(url.path(), "/api/search/nav");
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            vec![("q".into(), query.trim().into())]
        );
        assert!(url.fragment().is_none());
    }
    #[test]
    #[ignore = "contacts the live Reframed search endpoint"]
    fn live_site_search() {
        let hits = fetch_search("monet").unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(valid_artwork));
        assert!(
            fetch_search("zzzzzznonexistent123456789")
                .unwrap()
                .is_empty()
        );
    }
    fn flight(chunks: &[&str]) -> String {
        chunks
            .iter()
            .map(|s| {
                format!(
                    "<script>self.__next_f.push([1,{}])</script>",
                    serde_json::to_string(s).unwrap()
                )
            })
            .collect()
    }
    #[test]
    fn reconstructs_chunks_and_preserves_unicode() {
        let a = r#"{"id":"one","r2Key":"originals/Édouard Manet - A {study}.jpg","alt":"A {study}","href":"/artwork/one"}"#;
        let split = a.find("Manet").unwrap() + 2;
        let items = parse_catalogue(&flight(&[&a[..split], &a[split..]])).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].artist(), "Édouard Manet");
        assert!(items[0].original_url().contains("%C3%89douard%20Manet"));
        assert_eq!(
            url::Url::parse(&items[0].original_url())
                .unwrap()
                .host_str(),
            Some("cdn.reframed.gallery")
        );
    }
    #[test]
    fn rejects_untrusted_metadata_and_deduplicates() {
        let valid = r#"{"id":"a","r2Key":"originals/Artist - Title.jpg","alt":"Title","href":"/artwork/a"}"#;
        let external = valid.replace("/artwork/a", "https://evil.example/a");
        let traversal = valid.replace("originals/Artist", "originals/../Artist");
        let payload = format!("[{valid},{valid},{external},{traversal}]");
        assert_eq!(parse_catalogue(&flight(&[&payload])).unwrap().len(), 1);
        assert!(parse_catalogue("<html>no catalogue</html>").is_err());
    }
    #[test]
    fn recognizes_only_structured_empty_catalogue() {
        let empty =
            r#"["$","div",null,{"className":"TileGrid-module__wREHvq__grid","children":[]}]"#;
        assert!(parse_catalogue(&flight(&[empty])).unwrap().is_empty());
        assert!(parse_catalogue(&flight(&[r#"{"children":[]}"#])).is_err());
        let malformed = empty.replace("[]}", "[{}]}");
        assert!(parse_catalogue(&flight(&[&malformed])).is_err());
        assert!(parse_catalogue("self.__next_f.push([1,broken])").is_err());
    }
    #[test]
    fn only_later_page_not_found_confirms_end() {
        assert!(is_pagination_end(2, 404));
        assert!(!is_pagination_end(1, 404));
        assert!(!is_pagination_end(2, 500));
        assert!(!is_pagination_end(2, 403));
        assert!(!is_pagination_end(2, 429));
    }
}
