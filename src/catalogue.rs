use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const SITE: &str = "https://www.reframed.gallery";
#[derive(Clone, Debug, Deserialize, Serialize)]
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
        .set("User-Agent", "Mozilla/5.0 (Pinacora desktop)")
        .set("Referer", SITE)
        .call()?;
    read_text(response)
}
fn read_text(response: ureq::Response) -> Result<String> {
    let mut reader = response.into_reader().take(8 * 1024 * 1024 + 1);
    let mut body = String::new();
    std::io::Read::read_to_string(&mut reader, &mut body)?;
    if body.len() > 8 * 1024 * 1024 {
        bail!("Catalogue response too large");
    }
    Ok(body)
}
use std::io::Read;
#[derive(Clone, Debug)]
pub struct Page {
    pub artworks: Vec<Artwork>,
    pub last_page: Option<usize>,
}
pub fn fetch_page(page: usize) -> Result<Vec<Artwork>> {
    Ok(fetch_page_info(page)?.artworks)
}
pub fn fetch_page_info(page: usize) -> Result<Page> {
    anyhow::ensure!(
        (1..=1000).contains(&page),
        "Catalogue page is outside the safe range"
    );
    let path = if page == 1 {
        format!("{SITE}/recent")
    } else {
        format!("{SITE}/recent/page/{page}")
    };
    match agent()
        .get(&path)
        .set("User-Agent", "Mozilla/5.0 (Pinacora desktop)")
        .set("Referer", SITE)
        .call()
    {
        Ok(response) => page_response(page, response),
        Err(error) => {
            if let ureq::Error::Status(status, _) = &error
                && is_pagination_end(page, *status)
            {
                return Ok(Page {
                    artworks: vec![],
                    last_page: None,
                });
            }
            Err(error.into())
        }
    }
}
fn terminal_redirect(page: usize, status: u16, location: Option<&str>) -> Option<usize> {
    if page <= 1 || !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let location = location?;
    // Accept only canonical same-site pagination paths. Do not normalize dot
    // segments, follow a redirect, or accept credentials, ports, queries or fragments.
    let number = location
        .strip_prefix("/recent/page/")
        .or_else(|| location.strip_prefix("https://www.reframed.gallery/recent/page/"))?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let previous = number.parse::<usize>().ok()?;
    (previous > 0 && previous == page - 1 && number == previous.to_string()).then_some(previous)
}
fn page_response(page: usize, response: ureq::Response) -> Result<Page> {
    let status = response.status();
    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        if let Some(previous) = terminal_redirect(page, status, response.header("Location")) {
            return Ok(Page {
                artworks: vec![],
                last_page: Some(previous),
            });
        }
        bail!("Unexpected catalogue redirect on page {page}; the redirect was not followed");
    }
    anyhow::ensure!(
        (200..300).contains(&status),
        "Unexpected catalogue response {status} on page {page}"
    );
    parse_page_info(&read_text(response)?)
}
fn pagination_page(href: &str) -> Option<usize> {
    let url = url::Url::parse(SITE).ok()?.join(href).ok()?;
    if url.scheme() != "https"
        || url.host_str() != Some("www.reframed.gallery")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let number = url.path().strip_prefix("/recent/page/")?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let page = number.parse::<usize>().ok()?;
    (1..=1000).contains(&page).then_some(page)
}
/// Pagination is read only from real href attributes or decoded Next flight hrefs.
/// It is a discovery hint, not proof that all pages have been enumerated.
pub fn parse_page_info(html: &str) -> Result<Page> {
    let artworks = parse_catalogue(html)?;
    let mut last_page = None;
    let mut record = |href: &str| {
        if let Some(page) = pagination_page(href) {
            last_page = Some(last_page.map_or(page, |last: usize| last.max(page)));
        }
    };
    for marker in ["href=\"", "href='"] {
        let quote = if marker.ends_with('\"') { '\"' } else { '\'' };
        for tail in html.split(marker).skip(1) {
            if let Some((href, _)) = tail.split_once(quote) {
                record(href);
            }
        }
    }
    let mut flight = String::new();
    for chunk in html.split("self.__next_f.push([1,").skip(1) {
        if let Some(Ok(s)) = serde_json::Deserializer::from_str(chunk)
            .into_iter::<String>()
            .next()
        {
            flight.push_str(&s);
        }
    }
    for tail in flight.split("\"href\"").skip(1) {
        if let Some(tail) = tail.trim_start().strip_prefix(':')
            && let Some(Ok(href)) = serde_json::Deserializer::from_str(tail.trim_start())
                .into_iter::<String>()
                .next()
        {
            record(&href);
        }
    }
    Ok(Page {
        artworks,
        last_page,
    })
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
pub fn valid_artwork(item: &Artwork) -> bool {
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
    fn redirect_response(location: &str) -> ureq::Response {
        use std::str::FromStr;
        ureq::Response::from_str(&format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
        ))
        .unwrap()
    }
    #[test]
    fn terminal_redirect_confirms_only_immediate_previous_same_site_page() {
        for status in [301, 302, 303, 307, 308] {
            assert_eq!(
                terminal_redirect(59, status, Some("/recent/page/58")),
                Some(58)
            );
            assert_eq!(
                terminal_redirect(
                    59,
                    status,
                    Some("https://www.reframed.gallery/recent/page/58")
                ),
                Some(58)
            );
        }
        for location in [
            "https://evil.example/recent/page/58",
            "/recent/page/57",
            "/recent/page/59",
            "/recent/page/60",
            "/recent",
            "/recent/page/58?x=1",
            "/recent/page/58#x",
            "https://user@www.reframed.gallery/recent/page/58",
            "https://www.reframed.gallery:443/recent/page/58",
            "/recent/page/a/../58",
            "/recent/page/058",
            "/recent/page/0",
        ] {
            assert!(
                terminal_redirect(59, 307, Some(location)).is_none(),
                "{location}"
            );
            assert!(
                page_response(59, redirect_response(location)).is_err(),
                "{location}"
            );
        }
        assert!(terminal_redirect(1, 307, Some("/recent/page/58")).is_none());
        assert!(terminal_redirect(59, 200, Some("/recent/page/58")).is_none());
        assert!(terminal_redirect(59, 307, None).is_none());
        let end = page_response(59, redirect_response("/recent/page/58")).unwrap();
        assert!(end.artworks.is_empty());
        assert_eq!(end.last_page, Some(58));
    }
    #[test]
    fn collection_stops_at_confirmed_terminal_redirect_without_following() {
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut requests = vec![];
        let items = crate::rotation_catalogue::collect_with(
            |page| {
                requests.push(page);
                if page == 3 {
                    return page_response(page, redirect_response("/recent/page/2"));
                }
                Ok(Page {
                    artworks: vec![Artwork {
                        id: page.to_string(),
                        key: format!("originals/Artist - {page}.jpg"),
                        alt: page.to_string(),
                        href: format!("/artwork/{page}"),
                    }],
                    last_page: Some(2),
                })
            },
            &cancel,
            &std::collections::BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(requests, [1, 2, 3]);
    }
    #[test]
    fn pagination_accepts_only_bounded_same_site_recent_links() {
        for href in [
            "/recent/page/58",
            "https://www.reframed.gallery/recent/page/58",
        ] {
            assert_eq!(pagination_page(href), Some(58));
        }
        for href in [
            "/recent/page/0",
            "/recent/page/1001",
            "/recent/page/-1",
            "/recent/page/2?x=1",
            "/recent/page/2/",
            "https://evil.example/recent/page/58",
            "/artwork/58",
            "/recent/page/no",
        ] {
            assert!(pagination_page(href).is_none(), "{href}");
        }
        let valid = r#"{"id":"a","r2Key":"originals/Artist - Title.jpg","alt":"Title","href":"/artwork/a"}"#;
        let nav = r#"{"href":"/recent/page/58"}"#;
        let html = format!(
            "{}<a href=\"/recent/page/2\">Next</a>",
            flight(&[valid, nav])
        );
        let page = parse_page_info(&html).unwrap();
        assert_eq!(page.last_page, Some(58));
        assert_eq!(page.artworks.len(), 1);
    }
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
