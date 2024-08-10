// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// TODO: Better thread management during Builder.
// TODO: Better caching.


mod scrapingfunc;
pub use crate::scrapingfunc::basic_scraping;
pub use crate::scrapingfunc::commands_scraping;

mod torrentfunc;
pub use crate::torrentfunc::torrent_commands;
pub use crate::torrentfunc::torrent_functions;
pub use crate::torrentfunc::TorrentState;

mod custom_ui_automation;
mod mighty;


use core::str;
use std::error::Error;
use std::fs;
use serde::{Deserialize, Serialize};
use reqwest::Client;
use std::fs::File;
use std::io::Write;
use std::io::Read;
use std::fmt;
use std::thread;
use tauri::{Manager, Window};
use std::time::{Duration, Instant};
use tokio::time::timeout;
use std::path::PathBuf;
// use serde_json::json;
use std::path::Path;
// crates for requests
use reqwest;
use kuchiki::traits::*;
use anyhow::{Result, Context};
// stop threads
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
// caching
use std::num::NonZeroUsize;
use lru::LruCache;
use tokio::sync::Mutex;
use tauri::State;
// torrenting
use librqbit::Session;
use tauri::async_runtime;
use lazy_static::lazy_static;

lazy_static! {
    static ref SESSION: Mutex<Option<Arc<Session>>> = Mutex::new(None);
}

// Define a shared boolean flag
static STOP_FLAG: AtomicBool = AtomicBool::new(false);
static PAUSE_FLAG: AtomicBool = AtomicBool::new(false);




#[derive(Debug, Serialize, Deserialize)]
struct Game {
    title: String,
    img: String,
    desc: String,
    magnetlink: String,
    href: String
}

#[derive(Debug, Serialize, Deserialize)]
struct SingleGame {
    my_all_images: Vec<String>,
}



#[derive(Debug, Serialize, Deserialize)]
struct GameImages {
    my_all_images: Vec<String>,
}



async fn download_sitemap(app_handle: tauri::AppHandle, url: &str, filename: &str) -> Result<(), Box<dyn Error>> {

    let client = reqwest::Client::new();

    let mut response = client.get(url).send().await?;

    let mut binding = app_handle.path_resolver().app_data_dir().unwrap();
            
    binding.push("sitemaps");

    match Path::new(&binding).exists() {
        true => {
            ()
        }
        false => {
            fs::create_dir_all(&binding)?;
        },
    }

    // Create the files by joining current directory, relative path, and filename
    let file_path = &binding
        .join(format!("{}.xml", filename));    
    
    // Open a file at the specified path for writing
    let mut file = fs::File::create(&file_path).unwrap();
    
    // Asynchronously copy the response body to the file
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk).unwrap();
    }

    Ok(())
}

#[tokio::main]
async fn get_sitemaps_website(app_handle: tauri::AppHandle) -> Result<(), Box<dyn Error>> {

    println!("Before Sitemaps Request");

    for page_number in 1..=5 {

        let sitemap_number: Option<i32> = if page_number == 0 {
            None
        } else {
            Some(page_number)
        };

        let relative_url = if let Some(num) = sitemap_number {
            format!("https://fitgirl-repacks.site/post-sitemap{}.xml", num)
        } else {
            "https://fitgirl-repacks.site/post-sitemap/".to_string()
        };


        let relative_filename = format!("post-sitemap{}", if let Some(num) = sitemap_number {
            num.to_string()
        } else {
            "".to_string()
        });

        println!("relative url :  {}. relative filename: {}", relative_url, relative_filename);
        let my_app_handle = app_handle.clone();
        download_sitemap(my_app_handle, &relative_url, &relative_filename).await?;


    }


    Ok(())
}


fn extract_hrefs_from_body(body: &str) -> Result<Vec<String>> {
    let document = kuchiki::parse_html().one(body);
    let mut hrefs = Vec::new();
    let mut p_index = 3;

    while p_index < 10 {
        let href_selector_str = format!(".entry-content > p:nth-of-type({}) a[href]", p_index);

        for anchor_elem in document
            .select(&href_selector_str)
            .map_err(|_| anyhow::anyhow!("Failed to select anchor element"))?
        {
            if let Some(href_link) = anchor_elem.attributes.borrow().get("href") {
                hrefs.push(href_link.to_string());
            }
        }

        p_index += 1;
    }

    Ok(hrefs)
}

async fn fetch_and_process_href(client: &Client, href: &str) -> Result<Vec<String>> {
    let processing_time = Instant::now();

    if STOP_FLAG.load(Ordering::Relaxed) {
        return Err(anyhow::anyhow!("Cancelled the Event..."));
    }

    let mut image_srcs = Vec::new();
    let image_selector = "div.big-image > a > img";
    let noscript_selector = "noscript";
    println!("Start getting images process");

    let href_res = client
        .get(href)
        .send()
        .await
        .context("Failed to send HTTP request to HREF")?;
    if !href_res.status().is_success() {
        return Ok(image_srcs);
    }

    if STOP_FLAG.load(Ordering::Relaxed) {
        return Err(anyhow::anyhow!("Cancelled the Event..."));
    }

    let href_body = href_res.text().await.context("Failed to read HREF response body")?;
    let href_document = kuchiki::parse_html().one(href_body);

    println!("Start getting text process");

    if STOP_FLAG.load(Ordering::Relaxed) {
        return Err(anyhow::anyhow!("Cancelled the Event..."));
    }

    for noscript in href_document
        .select(noscript_selector)
        .map_err(|_| anyhow::anyhow!("Failed to select noscript element"))?
    {
        let inner_noscript_html = noscript.text_contents();
        let inner_noscript_document = kuchiki::parse_html().one(inner_noscript_html);

        for img_elem in inner_noscript_document
            .select(image_selector)
            .map_err(|_| anyhow::anyhow!("Failed to select image element"))?
        {
            if let Some(src) = img_elem.attributes.borrow().get("src") {
                image_srcs.push(src.to_string());
            }
        }

        // Check if the processing time exceeds 4 seconds
        if processing_time.elapsed() > Duration::new(4, 0) {
            println!("Processing time exceeded 4 seconds, returning collected images so far");
            return Ok(image_srcs);
        }
    }

    Ok(image_srcs)
}

async fn scrape_image_srcs(url: &str) -> Result<Vec<String>> {
    if STOP_FLAG.load(Ordering::Relaxed) {
        return Err(anyhow::anyhow!("Cancelled the Event..."));
    }

    let client = Client::new();
    let res = client
        .get(url)
        .send()
        .await
        .context("Failed to send HTTP request")?;

    if !res.status().is_success() {
        return Err(anyhow::anyhow!("Failed to connect to the website or the website is down."));
    }

    let body = res.text().await.context("Failed to read response body")?;
    println!("Start extracting hrefs");
    let hrefs = extract_hrefs_from_body(&body)?;

    let mut image_srcs = Vec::new();

    for href in hrefs {
        println!("Start fetching process");
        let result = timeout(Duration::new(4, 0), fetch_and_process_href(&client, &href)).await;
        match result {
            Ok(Ok(images)) => {
                if !images.is_empty() {
                    image_srcs.extend(images);
                }
            },
            Ok(Err(e)) => println!("Error fetching images from href: {}", e),
            Err(_) => println!("Timeout occurred while fetching images from href"),
        }
    }

    Ok(image_srcs)
}

// Cache with a capacity of 100
type ImageCache = Arc<Mutex<LruCache<String, Vec<String>>>>;

#[tauri::command]
async fn stop_get_games_images() {
    STOP_FLAG.store(true, Ordering::Relaxed);
}

#[tauri::command]
async fn get_games_images(game_link: String, image_cache: State<'_, ImageCache>) -> Result<(), CustomError> {
    STOP_FLAG.store(false, Ordering::Relaxed);
    let start_time = Instant::now();

    let mut cache = image_cache.lock().await;

    if let Some(cached_images) = cache.get(&game_link) {
        println!("Cache hit! Returning cached images.");
        let game = GameImages { my_all_images: cached_images.clone() };
        let json_data = serde_json::to_string_pretty(&game).context("Failed to serialize image sources to JSON")
            .map_err(|e| CustomError { message: e.to_string() })?;
        fs::write("../src/temp/singular_games.json", json_data).context("Failed to write JSON data to file")
            .map_err(|e| CustomError { message: e.to_string() })?;

        return Ok(());
    }

    drop(cache); // Release the lock before making network requests

    if STOP_FLAG.load(Ordering::Relaxed) {
        return Err(CustomError { message: "Function stopped.".to_string() });
    }

    let image_srcs = scrape_image_srcs(&game_link).await.map_err(|e| CustomError { message: e.to_string() })?;

    // Ensure that at least one image URL is included, even if no images were found
    let game = GameImages { my_all_images: image_srcs.clone() };
    let json_data = serde_json::to_string_pretty(&game).context("Failed to serialize image sources to JSON")
        .map_err(|e| CustomError { message: e.to_string() })?;
    fs::write("../src/temp/singular_games.json", json_data).context("Failed to write JSON data to file")
        .map_err(|e| CustomError { message: e.to_string() })?;

    if STOP_FLAG.load(Ordering::Relaxed) {
        return Err(CustomError { message: "Function stopped.".to_string() });
    }

    let end_time = Instant::now();
    let duration = end_time.duration_since(start_time);
    println!("Data has been written to single_games.json. Time was: {:?}", duration);

    // Update the cache
    let mut cache = image_cache.lock().await;
    cache.put(game_link, image_srcs);

    Ok(())
}

//Always serialize returns...
#[derive(Debug, Serialize, Deserialize)]
struct FileContent {
    content: String
}

#[derive(Debug, Serialize)]
struct CustomError {
    message: String,
}

impl fmt::Display for CustomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for CustomError {}

impl From<Box<dyn Error>> for CustomError {
    fn from(error: Box<dyn Error>) -> Self {
        CustomError {
            message: error.to_string(),
        }
    }
}



#[tauri::command(async)]
async fn read_file(file_path: String) -> Result<FileContent, CustomError> {
    let mut file = File::open(&file_path)
        .map_err(|e| CustomError { message: e.to_string() })?;
    let mut data_content = String::new();
    file.read_to_string(&mut data_content)
        .map_err(|e| CustomError { message: e.to_string() })?;

    Ok(FileContent { content: data_content })
}


#[tauri::command]
async fn clear_file(file_path: String) -> Result<(), CustomError> {
    let path = Path::new(&file_path);

    // Attempt to create the file, truncating if it already exists
    File::create(&path).map_err(|err| CustomError{ message: err.to_string()})?;
    
    Ok(())
}


#[tauri::command]
async fn close_splashscreen(window: Window) {
  // Close splashscreen
  window.get_window("splashscreen").expect("no window labeled 'splashscreen' found").close().unwrap();
  // Show main window
  window.get_window("main").expect("no window labeled 'main' found").show().unwrap();
}


#[tauri::command]
fn check_folder_path(path: String) -> Result<bool, bool> {
    let path_obj = PathBuf::from(&path);
    
    // Debugging information
    println!("Checking path: {:?}", path_obj);
    
    if !path_obj.exists() {
        println!("Path does not exist.");
        return Ok(false);
    }
    if !path_obj.is_dir() {
        println!("Path is not a directory.");
        return Ok(false);
    }
    println!("Path is valid.");
    Ok(true)
}





fn main() -> Result<(), Box<dyn Error>> {

    let image_cache = Arc::new(Mutex::new(LruCache::<String, Vec<String>>::new(NonZeroUsize::new(30).unwrap())));
    let torrent_state = torrentfunc::TorrentState::default();
    // let closing_signal_received = Arc::new(AtomicBool::new(false));


    tauri::Builder::default()
        .setup(move |app| {
            let current_app_handle: tauri::AppHandle = app.app_handle();

            // Only way I got it working, it is a performance nightmare please fix it. :(
            let first_app_handle = current_app_handle.clone();
            let second_app_handle = current_app_handle.clone();
            let third_app_handle = current_app_handle.clone();
            let fourth_app_handle = current_app_handle.clone();

            // Create a thread for the first function
            let handle1 = thread::Builder::new().name("scraping_func".into()).spawn(move || {
                if let Err(e) = basic_scraping::scraping_func(first_app_handle) {
                    eprintln!("Error in scraping_func: {}", e);
                    std::process::exit(1);
                }
            }).expect("Failed to spawn thread for scraping_func");

            // Create a thread for the second function
            let handle2 = thread::Builder::new().name("popular_and_recent_games_scraping_func".into()).spawn(|| {
                if let Err(e) = basic_scraping::popular_games_scraping_func(second_app_handle) {    
                    eprintln!("Error in popular_games_scraping_func: {}", e);
                    std::process::exit(1);
                }

                if let Err(e) = get_sitemaps_website(fourth_app_handle) {
                    eprintln!("Error in get_sitemaps_website: {}", e);
                    std::process::exit(1);
                }

                if let Err(e) = basic_scraping::recently_updated_games_scraping_func(third_app_handle) {
                    eprintln!("Error in recently_updated_games_scraping_func: {}", e);
                    std::process::exit(1);
                }
            }).expect("Failed to spawn thread for popular_games_scraping_func");

            // Wait for both threads to finish
            async_runtime::spawn(async move {
                handle1.join().expect("Thread 1 panicked");
                handle2.join().expect("Thread 2 panicked");
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            read_file,
            close_splashscreen,
            get_games_images,
            clear_file,
            stop_get_games_images,
            check_folder_path,
            torrent_commands::start_torrent_command,
            torrent_commands::get_torrent_stats,
            torrent_commands::stop_torrent_command,
            torrent_commands::pause_torrent_command,
            torrent_commands::resume_torrent_command,
            torrent_commands::select_files_to_download,
            commands_scraping::get_singular_game_info
        ])
        .manage(image_cache) // Make the cache available to commands 
        .manage(torrent_state) 
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, event| match event {
          tauri::RunEvent::ExitRequested { .. } => {
            
            PAUSE_FLAG.store(true, Ordering::Relaxed);
          }
          _ => {}
        });
    Ok(())
}