use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    sync::{
        LazyLock, Mutex,
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use image::imageops::FilterType;
use winit::event_loop::EventLoopProxy;

use crate::{app::UserEvent, git::cache::AvatarCache};

/// Canonical avatar fetch/atlas size; every surface scales this at draw time.
const AVATAR_SIZE: u32 = 64;
/// Cooldown before a failed avatar fetch is attempted again.
const RETRY_COOLDOWN: Duration = Duration::from_secs(30);
/// GitHub rejects API requests without a User-Agent.
const USER_AGENT: &str = concat!("kraken-native/", env!("CARGO_PKG_VERSION"));

#[derive(Default)]
struct AvatarStore {
    pixels: HashMap<String, Vec<u8>>,
    pending_identicons: HashMap<String, Vec<u8>>,
    versions: HashMap<String, u64>,
    next_version: u64,
    queued: HashSet<String>,
    /// Keys whose last fetch failed, holding identicon pixels until the
    /// cooldown elapses and a surface requests them again.
    retry_after: HashMap<String, Instant>,
}

struct Request {
    key: String,
    email: String,
}

static STORE: LazyLock<Mutex<AvatarStore>> = LazyLock::new(|| Mutex::new(AvatarStore::default()));
static REQUESTS: LazyLock<Sender<Request>> = LazyLock::new(start_workers);
static EVENT_LOOP_PROXY: LazyLock<Mutex<Option<EventLoopProxy<UserEvent>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Cross-session avatar storage; absent when the cache cannot be opened, in
/// which case every session refetches.
static CACHE: LazyLock<Option<AvatarCache>> = LazyLock::new(AvatarCache::platform);

/// `owner/repo` for the active repository when it lives on GitHub; lets a
/// commit email resolve to the account avatar GitHub keeps mapped privately.
static GITHUB_SLUG: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));

/// Points email resolution at the repository's GitHub remote, if it has one.
///
/// Passing a non-GitHub or absent remote clears the mapping and leaves
/// Gravatar as the only network source.
pub(crate) fn set_github_remote(remote_url: Option<&str>) {
    let slug = remote_url.and_then(github_slug);
    let mut current = GITHUB_SLUG.lock().expect("avatar slug lock");
    if *current == slug {
        return;
    }
    *current = slug;
    drop(current);
    // A different repository can resolve emails the previous one could not.
    // The deadlines are expired rather than removed: `retry_after` also marks
    // which keys still hold a placeholder, so dropping the entries would make
    // `request` treat those identicons as finished artwork.
    let now = Instant::now();
    for deadline in STORE
        .lock()
        .expect("avatar store lock")
        .retry_after
        .values_mut()
    {
        *deadline = now;
    }
}

/// Connects avatar fetch completions to the native event loop.
pub(crate) fn set_event_loop_proxy(proxy: EventLoopProxy<UserEvent>) {
    *EVENT_LOOP_PROXY.lock().expect("avatar proxy lock") = Some(proxy);
}

/// Queues retrieval of an author avatar and returns its stable atlas key.
pub(crate) fn request(email: &str) -> String {
    let email = email.trim().to_lowercase();
    let key = format!("{:x}", md5::compute(email.as_bytes()));
    let mut store = STORE.lock().expect("avatar store lock");
    let fetched = store.pixels.contains_key(&key) && !store.retry_after.contains_key(&key);
    let cooling = store
        .retry_after
        .get(&key)
        .is_some_and(|at| Instant::now() < *at);
    if !fetched && !cooling && store.queued.insert(key.clone()) {
        let _ = REQUESTS.send(Request {
            key: key.clone(),
            email,
        });
    }
    key
}

/// Returns RGBA pixels for an avatar, using a deterministic identicon while a fetch is pending.
pub(crate) fn pixels(key: &str) -> Vec<u8> {
    versioned_pixels(key).1
}

/// Returns the current pixel revision and RGBA payload for one avatar.
pub(crate) fn versioned_pixels(key: &str) -> (u64, Vec<u8>) {
    let mut store = STORE.lock().expect("avatar store lock");
    let pixels = if let Some(pixels) = store.pixels.get(key) {
        pixels.clone()
    } else {
        store
            .pending_identicons
            .entry(key.to_owned())
            .or_insert_with(|| identicon(key))
            .clone()
    };
    let version = store.versions.get(key).copied().unwrap_or_else(|| {
        store.next_version = store.next_version.wrapping_add(1);
        let version = store.next_version;
        store.versions.insert(key.to_owned(), version);
        version
    });
    (version, pixels)
}

fn start_workers() -> Sender<Request> {
    let (sender, receiver) = mpsc::channel::<Request>();
    let receiver = std::sync::Arc::new(Mutex::new(receiver));
    for index in 0..2 {
        let receiver = std::sync::Arc::clone(&receiver);
        thread::Builder::new()
            .name(format!("avatar-fetch-{index}"))
            .spawn(move || {
                loop {
                    let request = receiver.lock().expect("avatar receiver lock").recv();
                    let Ok(request) = request else { break };
                    fetch(request);
                }
            })
            .expect("start avatar fetch worker");
    }
    sender
}

fn fetch(request: Request) {
    let bytes = cache_load(&request.key).or_else(|| {
        // Gravatar first: an explicitly configured avatar outranks whatever
        // account happens to own the address. GitHub answers for the many
        // emails registered to an account but never published to Gravatar.
        let bytes = download(&avatar_url(&request.email, &request.key)).or_else(|| {
            let url = github_commit_avatar(&request.email)?;
            download(&url)
        })?;
        cache_store(&request.key, &bytes);
        Some(bytes)
    });
    let fetched = bytes.and_then(|bytes| decode_circle(&bytes));
    let mut store = STORE.lock().expect("avatar store lock");
    match fetched {
        Some(pixels) => {
            // A completed fetch replaces the placeholder for every surface
            // drawing this key; the renderer re-reads pixels each frame.
            store.pixels.insert(request.key.clone(), pixels);
            store.retry_after.remove(&request.key);
            store.pending_identicons.remove(&request.key);
        }
        None => {
            // Keep the identicon placeholder but allow a retry after the
            // cooldown instead of poisoning the store forever.
            let placeholder = store
                .pending_identicons
                .remove(&request.key)
                .unwrap_or_else(|| identicon(&request.key));
            store
                .pixels
                .entry(request.key.clone())
                .or_insert(placeholder);
            store
                .retry_after
                .insert(request.key.clone(), Instant::now() + RETRY_COOLDOWN);
        }
    }
    store.next_version = store.next_version.wrapping_add(1);
    let version = store.next_version;
    store.versions.insert(request.key.clone(), version);
    store.queued.remove(&request.key);
    drop(store);
    if let Some(proxy) = EVENT_LOOP_PROXY.lock().expect("avatar proxy lock").as_ref() {
        let _ = proxy.send_event(UserEvent::Avatar);
    }
}

fn avatar_url(email: &str, hash: &str) -> String {
    // GitHub noreply forms: "12345+login@…" (id-addressable) and the legacy
    // bare "login@…" (username-addressable).
    if let Some(user) = email.strip_suffix("@users.noreply.github.com") {
        if let Some((id, _)) = user.split_once('+')
            && id.chars().all(|character| character.is_ascii_digit())
        {
            return format!("https://avatars.githubusercontent.com/u/{id}?s={AVATAR_SIZE}");
        }
        if !user.is_empty() && !user.contains('+') {
            return format!("https://avatars.githubusercontent.com/{user}?s={AVATAR_SIZE}");
        }
    }
    // `d=404` instead of a generated fallback so an unknown address falls
    // through to GitHub resolution rather than locking in Gravatar's identicon.
    format!("https://www.gravatar.com/avatar/{hash}?d=404&s={AVATAR_SIZE}")
}

/// Fetches one image, treating any transport or non-2xx response as absent.
fn download(url: &str) -> Option<Vec<u8>> {
    let mut response = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .ok()?;
    response.body_mut().read_to_vec().ok()
}

/// Resolves a commit email to its GitHub account avatar via the active repo.
///
/// GitHub maps commit emails to accounts server-side, including addresses that
/// are registered but not public, which is exactly the set Gravatar and the
/// public user search both miss. Public repositories answer unauthenticated;
/// `GITHUB_TOKEN`/`GH_TOKEN` covers private repositories and rate limits.
fn github_commit_avatar(email: &str) -> Option<String> {
    let slug = GITHUB_SLUG.lock().expect("avatar slug lock").clone()?;
    let url = format!(
        "https://api.github.com/repos/{slug}/commits?per_page=1&author={}",
        percent_encode(email)
    );
    let mut request = ureq::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", USER_AGENT);
    if let Some(token) = github_token() {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    let mut response = request.call().ok()?;
    let commits: serde_json::Value = response.body_mut().read_json().ok()?;
    let avatar = commits
        .get(0)?
        .get("author")?
        .get("avatar_url")?
        .as_str()?
        .trim();
    // The API hands back a sizeable default; ask for the atlas size instead.
    (!avatar.is_empty()).then(|| format!("{avatar}&s={AVATAR_SIZE}"))
}

/// Optional GitHub credential, needed only for private repos and rate limits.
fn github_token() -> Option<String> {
    ["GITHUB_TOKEN", "GH_TOKEN"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

/// Extracts `owner/repo` from a GitHub remote in URL or scp-style form.
fn github_slug(remote_url: &str) -> Option<String> {
    let remote_url = remote_url.trim();
    let rest = ["https://", "http://", "ssh://", "git://"]
        .iter()
        .find_map(|scheme| remote_url.strip_prefix(scheme))
        .unwrap_or(remote_url);
    // Drops "git@" userinfo from both ssh URLs and scp-style paths.
    let rest = rest.split_once('@').map_or(rest, |(_, host)| host);
    let (host, path) = rest.split_once(['/', ':'])?;
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    (!owner.is_empty() && !repo.is_empty() && !repo.contains('/'))
        .then(|| format!("{owner}/{repo}"))
}

/// Percent-encodes a query value; emails carry `@` and `+`, which `+` in a
/// query string would otherwise decode as a space.
fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

/// Reads an encoded avatar image previously persisted for `key`.
fn cache_load(key: &str) -> Option<Vec<u8>> {
    CACHE.as_ref()?.load(key)
}

/// Persists an encoded avatar image; storage failures only cost a refetch.
fn cache_store(key: &str, bytes: &[u8]) {
    if let Some(cache) = CACHE.as_ref() {
        let _ = cache.store(key, bytes);
    }
}

fn decode_circle(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut pixels = image::load_from_memory(bytes)
        .ok()?
        .resize_exact(AVATAR_SIZE, AVATAR_SIZE, FilterType::Lanczos3)
        .to_rgba8()
        .into_raw();
    circle_mask(&mut pixels);
    Some(pixels)
}

fn identicon(key: &str) -> Vec<u8> {
    let digest = md5::compute(key.as_bytes());
    let color = [digest.0[0], digest.0[1], digest.0[2], 255];
    let mut pixels = vec![22; (AVATAR_SIZE * AVATAR_SIZE * 4) as usize];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    for row in 0..5 {
        for column in 0..3 {
            if digest.0[(row * 3 + column) as usize] & 1 == 0 {
                continue;
            }
            for mirror_column in [column, 4 - column] {
                let x0 = mirror_column * AVATAR_SIZE / 5;
                let x1 = (mirror_column + 1) * AVATAR_SIZE / 5;
                let y0 = row * AVATAR_SIZE / 5;
                let y1 = (row + 1) * AVATAR_SIZE / 5;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let offset = ((y * AVATAR_SIZE + x) * 4) as usize;
                        pixels[offset..offset + 4].copy_from_slice(&color);
                    }
                }
            }
        }
    }
    circle_mask(&mut pixels);
    pixels
}

fn circle_mask(pixels: &mut [u8]) {
    let center = (AVATAR_SIZE as f32 - 1.0) * 0.5;
    let radius = AVATAR_SIZE as f32 * 0.5;
    for y in 0..AVATAR_SIZE {
        for x in 0..AVATAR_SIZE {
            let distance = ((x as f32 - center).powi(2) + (y as f32 - center).powi(2)).sqrt();
            if distance > radius {
                pixels[((y * AVATAR_SIZE + x) * 4 + 3) as usize] = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_noreply_uses_account_avatar() {
        assert_eq!(
            avatar_url("12345+octocat@users.noreply.github.com", "x"),
            "https://avatars.githubusercontent.com/u/12345?s=64"
        );
    }

    #[test]
    fn legacy_github_noreply_uses_username_avatar() {
        assert_eq!(
            avatar_url("octocat@users.noreply.github.com", "x"),
            "https://avatars.githubusercontent.com/octocat?s=64"
        );
    }

    #[test]
    fn gravatar_declines_unknown_emails_so_github_can_answer() {
        assert_eq!(
            avatar_url("person@example.com", "abc"),
            "https://www.gravatar.com/avatar/abc?d=404&s=64"
        );
    }

    #[test]
    fn github_slug_accepts_every_remote_form() {
        for remote in [
            "https://github.com/can1357/kraken-rs.git",
            "https://github.com/can1357/kraken-rs",
            "git@github.com:can1357/kraken-rs.git",
            "ssh://git@github.com/can1357/kraken-rs.git",
            "https://GitHub.com/can1357/kraken-rs/",
        ] {
            assert_eq!(
                github_slug(remote).as_deref(),
                Some("can1357/kraken-rs"),
                "{remote}"
            );
        }
    }

    #[test]
    fn github_slug_rejects_other_hosts_and_partial_paths() {
        for remote in [
            "https://gitlab.com/can1357/kraken-rs.git",
            "git@bitbucket.org:can1357/kraken-rs.git",
            "https://github.com/can1357",
            "https://github.company.com/can1357/kraken-rs.git",
            "/local/path/repo.git",
        ] {
            assert_eq!(github_slug(remote), None, "{remote}");
        }
    }

    #[test]
    fn percent_encode_escapes_email_punctuation() {
        assert_eq!(percent_encode("a+b@can.ac"), "a%2Bb%40can.ac");
    }

    /// Switching repositories must give placeholder avatars another chance
    /// without erasing the mark that says they *are* placeholders — dropping
    /// the entry makes `request` mistake a stale identicon for real artwork
    /// and never refetch it.
    #[test]
    fn changing_repository_reopens_failed_avatars() {
        let key = "avatar-retry-fixture".to_owned();
        {
            let mut store = STORE.lock().expect("avatar store lock");
            store.pixels.insert(key.clone(), vec![0; 4]);
            store
                .retry_after
                .insert(key.clone(), Instant::now() + RETRY_COOLDOWN);
        }

        set_github_remote(None);
        set_github_remote(Some("https://github.com/can1357/kraken-rs.git"));

        let store = STORE.lock().expect("avatar store lock");
        let deadline = *store
            .retry_after
            .get(&key)
            .expect("placeholder mark must survive a repository change");
        assert!(
            deadline <= Instant::now(),
            "cooldown must be expired so the next request refetches"
        );
        assert!(
            store.pixels.contains_key(&key),
            "the identicon keeps painting until the refetch lands"
        );
    }
}
