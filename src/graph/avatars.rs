use std::{
    collections::{HashMap, HashSet},
    fs,
    sync::{
        LazyLock, Mutex,
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use directories::ProjectDirs;
use image::imageops::FilterType;
use winit::event_loop::EventLoopProxy;

use crate::app::UserEvent;

/// Canonical avatar fetch/atlas size; every surface scales this at draw time.
const AVATAR_SIZE: u32 = 64;
/// Cooldown before a failed avatar fetch is attempted again.
const RETRY_COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Default)]
struct AvatarStore {
    pixels: HashMap<String, Vec<u8>>,
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
    STORE
        .lock()
        .expect("avatar store lock")
        .pixels
        .get(key)
        .cloned()
        .unwrap_or_else(|| identicon(key))
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
    let path = cache_path(&request.key);
    let bytes = fs::read(&path).ok().or_else(|| {
        let url = avatar_url(&request.email, &request.key);
        let mut response = ureq::get(&url).call().ok()?;
        let bytes = response.body_mut().read_to_vec().ok()?;
        let _ = fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")));
        let _ = fs::write(&path, &bytes);
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
        }
        None => {
            // Keep the identicon placeholder but allow a retry after the
            // cooldown instead of poisoning the store forever.
            store
                .pixels
                .entry(request.key.clone())
                .or_insert_with(|| identicon(&request.key));
            store
                .retry_after
                .insert(request.key.clone(), Instant::now() + RETRY_COOLDOWN);
        }
    }
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
    format!("https://www.gravatar.com/avatar/{hash}?d=retro&s={AVATAR_SIZE}")
}

fn cache_path(key: &str) -> std::path::PathBuf {
    ProjectDirs::from("ac", "Kraken Native", "Kraken Native")
        .map(|dirs| dirs.cache_dir().join("avatars").join(format!("{key}.png")))
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("kraken-native-avatars")
                .join(format!("{key}.png"))
        })
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
    fn gravatar_url_matches_gitkraken_fallback() {
        assert_eq!(
            avatar_url("person@example.com", "abc"),
            "https://www.gravatar.com/avatar/abc?d=retro&s=64"
        );
    }
}
