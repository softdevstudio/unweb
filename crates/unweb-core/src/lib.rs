//! What Unweb counts for each stranger.
//!
//! This crate started as a copy of Watchkeeper's core crate
//! (softdevstudio/watchkeeper, crates/watchkeeper-core, commit ec99755,
//! Apache 2.0), taken once as a starter. It is Unweb's own code now and is
//! not kept in step with Watchkeeper. From that copy it keeps the idea of a
//! "site" and the cookie record; the cookie counter is new.
//!
//! A "site" is a registrable domain, so `news.example.co.uk` and
//! `cdn.example.co.uk` are one site. IP addresses and `localhost` are their
//! own sites.

use serde::{Deserialize, Serialize};
use url::Url;

/// A cookie as the engine's cookie store reports it. Names and domains
/// only, never values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCookie {
    pub name: String,
    pub domain: String,
    pub http_only: bool,
}

/// The site a host belongs to.
pub fn site_of_host(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || host == "localhost"
        || host.starts_with('[')
        || host.parse::<std::net::IpAddr>().is_ok()
    {
        return host;
    }
    match psl::domain_str(&host) {
        Some(d) => d.to_string(),
        None => host,
    }
}

/// The site of a URL, or `None` for URLs without a host.
pub fn site_of_url(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    let host = u.host_str()?;
    Some(site_of_host(host))
}

/// The cookies a stranger holds, counted against the site it is showing.
///
/// A stranger's store starts empty, so everything in it was handed to this
/// stranger since it opened: by the site it is on, or by other sites (its
/// own earlier pages, or third parties embedded in them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CookieCount {
    /// The site the stranger is showing; empty for a page with no host.
    pub site: String,
    /// Cookies whose domain belongs to that site.
    pub this_site: usize,
    /// Of those, how many scripts can't read (HttpOnly).
    pub this_site_http_only: usize,
    /// Cookies of every other site in the store.
    pub other_sites: usize,
    /// How many other sites those belong to.
    pub other_site_count: usize,
}

/// Counts `cookies` (the stranger's whole store) for the page at `page_url`.
pub fn count_cookies(page_url: &str, cookies: &[StoredCookie]) -> CookieCount {
    let site = site_of_url(page_url).unwrap_or_default();
    let mut count = CookieCount {
        site: site.clone(),
        ..Default::default()
    };
    let mut others = std::collections::BTreeSet::new();
    for c in cookies {
        let owner = site_of_host(c.domain.trim_start_matches('.'));
        if !site.is_empty() && owner == site {
            count.this_site += 1;
            if c.http_only {
                count.this_site_http_only += 1;
            }
        } else {
            count.other_sites += 1;
            others.insert(owner);
        }
    }
    count.other_site_count = others.len();
    count
}

/// A short line for the bar: "18 cookies from example.com, 3 from 2 other
/// sites".
pub fn describe(count: &CookieCount) -> String {
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let mut s = if count.site.is_empty() {
        "No site".to_string()
    } else {
        format!(
            "{} from {}",
            plural(count.this_site, "cookie", "cookies"),
            count.site
        )
    };
    if count.other_sites > 0 {
        s.push_str(&format!(
            ", {} from {}",
            count.other_sites,
            plural(count.other_site_count, "other site", "other sites")
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, domain: &str, http_only: bool) -> StoredCookie {
        StoredCookie {
            name: name.into(),
            domain: domain.into(),
            http_only,
        }
    }

    #[test]
    fn sites() {
        assert_eq!(site_of_host("news.bbc.co.uk"), "bbc.co.uk");
        assert_eq!(site_of_host("WWW.Example.COM."), "example.com");
        assert_eq!(site_of_host("localhost"), "localhost");
        assert_eq!(site_of_host("127.0.0.1"), "127.0.0.1");
        assert_eq!(
            site_of_url("https://cdn.example.com/x.js").as_deref(),
            Some("example.com")
        );
        assert_eq!(site_of_url("about:blank"), None);
    }

    #[test]
    fn counting() {
        let store = vec![
            c("a", ".example.com", false),
            c("b", "www.example.com", true),
            c("c", "shop.example.com", false),
            c("t", ".tracker.net", false),
            c("u", "ads.tracker.net", false),
            c("v", "cdn.other.org", true),
        ];
        let n = count_cookies("https://www.example.com/page?q=1", &store);
        assert_eq!(n.site, "example.com");
        assert_eq!(n.this_site, 3);
        assert_eq!(n.this_site_http_only, 1);
        assert_eq!(n.other_sites, 3);
        assert_eq!(n.other_site_count, 2);
        assert_eq!(
            describe(&n),
            "3 cookies from example.com, 3 from 2 other sites"
        );
    }

    #[test]
    fn a_site_that_only_looks_alike_is_another_site() {
        let store = vec![c("a", "example.com.evil.net", false)];
        let n = count_cookies("https://example.com/", &store);
        assert_eq!((n.this_site, n.other_sites), (0, 1));
    }

    #[test]
    fn empty_and_hostless() {
        assert_eq!(
            describe(&count_cookies("https://example.com/", &[])),
            "0 cookies from example.com"
        );
        let n = count_cookies("about:blank", &[c("a", "x.com", false)]);
        assert_eq!((n.this_site, n.other_sites), (0, 1));
        assert_eq!(describe(&n), "No site, 1 from 1 other site");
        assert_eq!(
            describe(&count_cookies(
                "http://127.0.0.1:8080/",
                &[c("a", "127.0.0.1", false)]
            )),
            "1 cookie from 127.0.0.1"
        );
    }
}
