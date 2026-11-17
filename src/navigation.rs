use crate::storage::SearchEngine;
use std::fmt;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationError {
    EmptyHost,
    InvalidUrl(String),
    UnsupportedScheme(String),
}

impl fmt::Display for NavigationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyHost => write!(formatter, "В адресе не указан хост."),
            Self::InvalidUrl(value) => write!(formatter, "Некорректный URL: {value}"),
            Self::UnsupportedScheme(scheme) => write!(
                formatter,
                "Схема {scheme} не поддерживается. Используйте HTTP или HTTPS."
            ),
        }
    }
}

#[cfg(test)]
pub fn normalize_input(input: &str) -> Result<Option<String>, NavigationError> {
    normalize_input_with(input, SearchEngine::Google)
}

pub fn normalize_input_with(
    input: &str,
    engine: SearchEngine,
) -> Result<Option<String>, NavigationError> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(None);
    }

    if let Some((scheme, _)) = input.split_once("://")
        && !scheme.eq_ignore_ascii_case("http")
        && !scheme.eq_ignore_ascii_case("https")
    {
        return Err(NavigationError::UnsupportedScheme(scheme.to_owned()));
    }

    if input.starts_with("http://") || input.starts_with("https://") {
        return parse_http_url(input).map(Some);
    }

    if looks_like_address(input) {
        return parse_http_url(&format!("https://{input}")).map(Some);
    }

    search_url(engine, input).map(Some)
}

pub fn search_url(engine: SearchEngine, input: &str) -> Result<String, NavigationError> {
    let base = match engine {
        SearchEngine::Google => "https://www.google.com/search",
        SearchEngine::Bing => "https://www.bing.com/search",
        SearchEngine::DuckDuckGo => "https://duckduckgo.com/",
        SearchEngine::Yandex => "https://yandex.com/search/",
    };
    let mut search = Url::parse(base).map_err(|_| NavigationError::InvalidUrl(input.to_owned()))?;
    search.query_pairs_mut().append_pair("q", input);
    Ok(search.into())
}

fn parse_http_url(value: &str) -> Result<String, NavigationError> {
    let url = Url::parse(value).map_err(|_| NavigationError::InvalidUrl(value.to_owned()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(NavigationError::UnsupportedScheme(url.scheme().to_owned()));
    }
    if url.host().is_none() {
        return Err(NavigationError::EmptyHost);
    }
    Ok(url.into())
}

fn looks_like_address(input: &str) -> bool {
    if input.chars().any(char::is_whitespace) {
        return false;
    }

    let without_path = input.split(['/', '?', '#']).next().unwrap_or_default();
    let host = without_path
        .strip_prefix('[')
        .and_then(|value| value.split(']').next())
        .unwrap_or_else(|| without_path.split(':').next().unwrap_or_default());

    host.eq_ignore_ascii_case("localhost")
        || host.parse::<std::net::IpAddr>().is_ok()
        || without_path.starts_with('[')
        || host.contains('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_https_to_domain() {
        assert_eq!(
            normalize_input("example.com/path").unwrap(),
            Some("https://example.com/path".to_owned())
        );
    }

    #[test]
    fn preserves_http_and_https() {
        assert_eq!(
            normalize_input("http://localhost:8080").unwrap(),
            Some("http://localhost:8080/".to_owned())
        );
        assert_eq!(
            normalize_input("https://example.com").unwrap(),
            Some("https://example.com/".to_owned())
        );
    }

    #[test]
    fn recognizes_local_addresses() {
        assert_eq!(
            normalize_input("localhost:3000").unwrap(),
            Some("https://localhost:3000/".to_owned())
        );
        assert_eq!(
            normalize_input("127.0.0.1:8000").unwrap(),
            Some("https://127.0.0.1:8000/".to_owned())
        );
    }

    #[test]
    fn turns_plain_text_into_search() {
        assert_eq!(
            normalize_input("rust language").unwrap(),
            Some("https://www.google.com/search?q=rust+language".to_owned())
        );
        assert_eq!(
            normalize_input("chromium").unwrap(),
            Some("https://www.google.com/search?q=chromium".to_owned())
        );
    }

    #[test]
    fn uses_the_selected_search_engine() {
        for (engine, expected) in [
            (SearchEngine::Bing, "https://www.bing.com/search?q=rust"),
            (SearchEngine::DuckDuckGo, "https://duckduckgo.com/?q=rust"),
            (SearchEngine::Yandex, "https://yandex.com/search/?q=rust"),
        ] {
            assert_eq!(
                normalize_input_with("rust", engine).unwrap(),
                Some(expected.to_owned())
            );
        }
    }

    #[test]
    fn handles_empty_and_unsupported_input() {
        assert_eq!(normalize_input("  ").unwrap(), None);
        assert!(matches!(
            normalize_input("file:///etc/passwd"),
            Err(NavigationError::UnsupportedScheme(_))
        ));
        assert!(normalize_input("https://").is_err());
    }
}
