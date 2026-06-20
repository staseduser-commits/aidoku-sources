#![no_std]
use aidoku::{
    AidokuError, Chapter, DeepLinkHandler, DeepLinkResult, FilterValue,
    Home, HomeComponent, HomeComponentValue, HomeLayout,
    Listing, ListingProvider, Manga, MangaPageResult,
    MangaStatus, Page, Result, Source,
    alloc::{String, Vec, format},
    imports::{html::Element, net::Request},
    prelude::*,
};

const BASE_URL: &str = "https://novelfire.net";

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(*byte as char),
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
                out.push(char::from_digit((byte & 0xf) as u32, 16).unwrap_or('0'));
            }
        }
    }
    out
}

fn abs(path: &str) -> String {
    if path.starts_with("http") {
        String::from(path)
    } else {
        format!("{}{}", BASE_URL, path)
    }
}

fn parse_novel_item(item: Element) -> Option<Manga> {
    let anchor = item.select_first("a")?;
    let id     = anchor.attr("href")?;
    let title  = anchor.select_first("h4.novel-title")
                        .and_then(|e| e.text())
                        .unwrap_or_default();
    let cover_path = anchor
        .select_first("figure.novel-cover img")
        .and_then(|img| {
            img.attr("src")
               .filter(|s| !s.contains("data:image"))
               .or_else(|| img.attr("data-src"))
        })
        .unwrap_or_default();

    Some(Manga {
        key: id,
        title,
        cover: Some(abs(&cover_path)),
        ..Default::default()
    })
}

fn chapter_number(id: &str) -> f32 {
    if let Some(seg) = id.split('/').last() {
        if let Some(rest) = seg.strip_prefix("chapter-") {
            if let Ok(n) = rest.replacen('-', ".", 1).parse::<f32>() {
                return n;
            }
            if let Ok(n) = rest.split('-').next().unwrap_or("0").parse::<f32>() {
                return n;
            }
        }
    }
    0.0
}

/// "2022-06-02 14:30:56" → Unix timestamp as i64
fn parse_datetime(dt: &str) -> Option<i64> {
    let dt = dt.trim();
    if dt.len() < 19 { return None; }
    let year:  i64 = dt[0..4].parse().ok()?;
    let month: i64 = dt[5..7].parse().ok()?;
    let day:   i64 = dt[8..10].parse().ok()?;
    let hour:  i64 = dt[11..13].parse().ok()?;
    let min:   i64 = dt[14..16].parse().ok()?;
    let sec:   i64 = dt[17..19].parse().ok()?;

    let mut days: i64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let md: [i64; 12] = [31, if is_leap(year) { 29 } else { 28 },
                          31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 0..(month - 1) as usize { days += md[m]; }
    days += day - 1;
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

fn is_leap(y: i64) -> bool { (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 }

// ─────────────────────────────────────────────────────────────────────────────
struct NovelFireSource;

impl Source for NovelFireSource {
    fn new() -> Self { NovelFireSource }

    fn get_search_manga_list(
        &self,
        query: Option<String>,
        page: i32,
        filters: Vec<FilterValue>,
    ) -> Result<MangaPageResult> {
        let url = if let Some(q) = query.filter(|s| !s.is_empty()) {
            format!("{}/search?keyword={}&page={}", BASE_URL, urlencode(&q), page)
        } else {
            let mut genre  = "all";
            let mut sort   = "new";
            let mut status = "all";
            // Filters arrive in order: index 0=Sort, 1=Status, 2=Genre
            if let Some(FilterValue::Select { value, .. }) = filters.get(0) {
                sort = match value.as_str() { "popular" => "popular", "rating" => "rating", _ => "new" };
            }
            if let Some(FilterValue::Select { value, .. }) = filters.get(1) {
                status = match value.as_str() { "ongoing" => "ongoing", "completed" => "completed", _ => "all" };
            }
            if let Some(FilterValue::Select { value, .. }) = filters.get(2) {
                genre = match value.as_str() {
                    "action"        => "action",
                    "adventure"     => "adventure",
                    "fantasy"       => "fantasy",
                    "romance"       => "romance",
                    "martial-arts"  => "martial-arts",
                    "sci-fi"        => "sci-fi",
                    "slice-of-life" => "slice-of-life",
                    "supernatural"  => "supernatural",
                    "comedy"        => "comedy",
                    "drama"         => "drama",
                    "horror"        => "horror",
                    "mystery"       => "mystery",
                    "josei"         => "josei",
                    "video-games"   => "video-games",
                    _               => "all",
                };
            }
            format!("{}/genre-{}/sort-{}/status-{}/all-novel?page={}", BASE_URL, genre, sort, status, page)
        };

        let html = Request::get(&url)?.html()?;
        let mut entries = Vec::new();
        if let Some(items) = html.select("li.novel-item") {
            for item in items {
                if let Some(m) = parse_novel_item(item) { entries.push(m); }
            }
        }
        let has_next = html
            .select_first("ul.pagination li.page-item:not(.disabled) a[rel=next]")
            .is_some();
        Ok(MangaPageResult { entries, has_next_page: has_next })
    }

    fn get_manga_update(
        &self,
        manga: Manga,
        needs_details: bool,
        needs_chapters: bool,
    ) -> Result<Manga> {
        let mut updated = manga.clone();

        if needs_details {
            let html = Request::get(&format!("{}{}", BASE_URL, manga.key))?.html()?;

            if let Some(el) = html.select_first("h1.novel-title") {
                if let Some(t) = el.text() { updated.title = t; }
            }
            updated.authors = html
                .select_first("div.author a[itemprop=author]")
                .and_then(|e| e.text())
                .map(|a| { let mut v = Vec::new(); v.push(a); v });
            updated.cover = html
                .select_first("meta[property='og:image']")
                .and_then(|e| e.attr("content"))
                .or_else(|| {
                    html.select_first("div.fixed-img figure.cover img")
                        .and_then(|e| e.attr("src"))
                        .map(|p| abs(&p))
                });
            updated.url = html
                .select_first("link[rel=canonical]")
                .and_then(|e| e.attr("href"));
            if let Some(paras) = html.select("div.summary div.content p") {
                let mut parts: Vec<String> = Vec::new();
                for p in paras {
                    if let Some(t) = p.text() {
                        if !t.trim().is_empty() { parts.push(t); }
                    }
                }
                if !parts.is_empty() {
                    updated.description = Some(parts.join("\n\n"));
                }
            }
            if let Some(links) = html.select("div.categories ul li a") {
                let mut tags: Vec<String> = Vec::new();
                for a in links {
                    if let Some(t) = a.text() { tags.push(t); }
                }
                updated.tags = Some(tags);
            }
            updated.status = html
                .select_first("div.header-stats strong")
                .and_then(|e| e.text())
                .map(|t| {
                    let l = t.to_lowercase();
                    if l.contains("ongoing")      { MangaStatus::Ongoing }
                    else if l.contains("complet") { MangaStatus::Completed }
                    else if l.contains("hiatus")  { MangaStatus::Hiatus }
                    else                          { MangaStatus::Unknown }
                })
                .unwrap_or(MangaStatus::Unknown);
        }

        if needs_chapters {
            let chapters_base = format!("{}{}/chapters", BASE_URL, manga.key);
            let mut all_chapters: Vec<Chapter> = Vec::new();
            let mut ch_page = 1;

            loop {
                let ch_html = match Request::get(&format!("{}?page={}", chapters_base, ch_page))
                    .and_then(|r| r.html()) {
                    Ok(h) => h,
                    Err(_) => break,
                };
                let rows = match ch_html.select("ul.chapter-list li") {
                    Some(r) => r,
                    None => break,
                };
                if rows.is_empty() { break; }

                for row in rows {
                    let anchor = match row.select_first("a") { Some(a) => a, None => continue };
                    let ch_key = match anchor.attr("href")   { Some(h) => h, None => continue };

                    let ch_num = anchor
                        .select_first("span.chapter-no")
                        .and_then(|e| e.text())
                        .and_then(|t| t.trim().parse::<f32>().ok())
                        .unwrap_or_else(|| chapter_number(&ch_key));

                    let ch_title = anchor
                        .select_first("strong.chapter-title")
                        .and_then(|e| e.text())
                        .or_else(|| anchor.attr("title"));

                    let date_uploaded = anchor
                        .select_first("time.chapter-update")
                        .and_then(|e| e.attr("datetime"))
                        .and_then(|dt| parse_datetime(&dt));

                    all_chapters.push(Chapter {
                        key: ch_key,
                        title: ch_title,
                        chapter_number: Some(ch_num),
                        volume_number: Some(-1.0),
                        date_uploaded,
                        scanlators: None,
                        url: None,
                        language: Some(String::from("en")),
                        thumbnail: None,
                        locked: false,
                    });
                }

                let more = ch_html
                    .select_first("ul.pagination li.page-item:not(.disabled) a[rel=next]")
                    .is_some();
                if !more { break; }
                ch_page += 1;
            }

            // Chapters from novelfire are listed oldest-first (ch1 → ch3050)
            // Aidoku expects newest-first so "Read" starts from the latest
            all_chapters.reverse();
            updated.chapters = Some(all_chapters);
        }

        Ok(updated)
    }

    fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let html = Request::get(&format!("{}{}", BASE_URL, chapter.key))?.html()?;
        let mut parts: Vec<String> = Vec::new();
        if let Some(paras) = html.select("div#content p") {
            for p in paras {
                if let Some(t) = p.text() {
                    if !t.trim().is_empty() { parts.push(t); }
                }
            }
        }
        if parts.is_empty() {
            return Err(AidokuError::Unimplemented);
        }
        let mut pages = Vec::new();
        pages.push(Page {
            content: aidoku::PageContent::Text(parts.join("\n\n")),
            thumbnail: None,
            has_description: false,
            description: None,
        });
        Ok(pages)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
impl Home for NovelFireSource {
    fn get_home(&self) -> Result<HomeLayout> {
        let mut components = Vec::new();

        // ── Latest Updates section ──
        let latest_url = format!("{}/genre-all/sort-new/status-all/all-novel?page=1", BASE_URL);
        if let Ok(html) = Request::get(&latest_url).and_then(|r| r.html()) {
            let mut entries = Vec::new();
            if let Some(items) = html.select("li.novel-item") {
                for item in items {
                    if let Some(m) = parse_novel_item(item) { entries.push(m.into()); }
                }
            }
            components.push(HomeComponent {
                title: Some(String::from("Latest Updates")),
                subtitle: None,
                value: HomeComponentValue::MangaList {
                    ranking: false,
                    page_size: None,
                    entries,
                    listing: Some(Listing {
                        id: String::from("latest"),
                        name: String::from("Latest Updates"),
                        kind: Default::default(),
                    }),
                },
            });
        }

        // ── Most Popular section ──
        let popular_url = format!("{}/genre-all/sort-popular/status-all/all-novel?page=1", BASE_URL);
        if let Ok(html) = Request::get(&popular_url).and_then(|r| r.html()) {
            let mut entries = Vec::new();
            if let Some(items) = html.select("li.novel-item") {
                for item in items {
                    if let Some(m) = parse_novel_item(item) { entries.push(m.into()); }
                }
            }
            components.push(HomeComponent {
                title: Some(String::from("Most Popular")),
                subtitle: None,
                value: HomeComponentValue::MangaList {
                    ranking: false,
                    page_size: None,
                    entries,
                    listing: Some(Listing {
                        id: String::from("popular"),
                        name: String::from("Most Popular"),
                        kind: Default::default(),
                    }),
                },
            });
        }

        Ok(HomeLayout { components })
    }
}

impl ListingProvider for NovelFireSource {
    fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
        let url = match listing.id.as_str() {
            "popular"   => format!("{}/genre-all/sort-popular/status-all/all-novel?page={}", BASE_URL, page),
            "completed" => format!("{}/genre-all/sort-popular/status-completed/all-novel?page={}", BASE_URL, page),
            _           => format!("{}/genre-all/sort-new/status-all/all-novel?page={}", BASE_URL, page),
        };
        let html = Request::get(&url)?.html()?;
        let mut entries = Vec::new();
        if let Some(items) = html.select("li.novel-item") {
            for item in items {
                if let Some(m) = parse_novel_item(item) { entries.push(m); }
            }
        }
        let has_next = html
            .select_first("ul.pagination li.page-item:not(.disabled) a[rel=next]")
            .is_some();
        Ok(MangaPageResult { entries, has_next_page: has_next })
    }
}

impl DeepLinkHandler for NovelFireSource {
    fn handle_deep_link(&self, _url: String) -> Result<Option<DeepLinkResult>> {
        Err(AidokuError::Unimplemented)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
register_source!(NovelFireSource, ListingProvider, Home, DeepLinkHandler);
