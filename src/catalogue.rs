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
    parse_catalogue(&get_text(&path)?)
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
    // Deserialize from each opening brace: serde reads exactly one balanced object,
    // including braces and escaped quotes inside strings.
    for (offset, _) in flight.match_indices('{') {
        let mut de = serde_json::Deserializer::from_str(&flight[offset..]).into_iter::<Artwork>();
        if let Some(Ok(item)) = de.next()
            && !item.id.is_empty()
            && !item.key.is_empty()
            && !item.alt.is_empty()
            && item.key.starts_with("originals/")
            && !item
                .key
                .split('/')
                .any(|s| s == "." || s == ".." || s.is_empty())
            && item.page_url().is_ok()
            && seen.insert(item.id.clone())
        {
            found.push(item);
        }
    }
    if found.is_empty() {
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
}
