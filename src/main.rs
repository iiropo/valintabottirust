use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};
use chrono::{Local, NaiveTime};
use futures::stream::{self, StreamExt};
use reqwest::{Client, header::{HeaderMap, HeaderValue}};
use serde::Deserialize;
use tokio::time::sleep;

#[derive(Deserialize)]
struct Credentials {
    session_id: String,
    formkey: String,
    school_id: String,
}

#[derive(Deserialize)]
struct SelectConfig {
    time: String,
    bypass: bool,
    target: Vec<String>,
}

pub async fn send_request(
    client: Arc<Client>,
    message_value: &str,
    formkey: &str,
    url: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let custom_body = format!(
        "message=pick-group&target={}&formkey={}",
        message_value, formkey
    );

    let response = client
        .post(url)
        .body(custom_body)
        .send()
        .await?;

    if response.status() != reqwest::StatusCode::OK {
        return Err(Box::from(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Got status: {}", response.status()),
        )));
    }

    let status = response.status(); // Extract the status before consuming the response
    let body = response.text().await?;
    if body.contains("srvError('Valintaa ei voi muuttaa', 'Ryhmä on jo valittu.');") {
        println!("Jo valittu ryhmä, ei tarvitse muuttaa.");
    }
    else if body.contains("srvError") {
        println!("Body: {}", body);
        return Err(Box::from(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Response contains SrvError",
        )));
    }

    println!("Status: {}", status); // Use the extracted status
    println!("Body: {}", body);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Read credentials from creds.json
    let mut file = File::open("src/creds.json")?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let creds: Credentials = serde_json::from_str(&contents)?;

    // Read config from select.json
    let mut select_file = File::open("src/select.json")?;
    let mut select_contents = String::new();
    select_file.read_to_string(&mut select_contents)?;
    let select_config: SelectConfig = serde_json::from_str(&select_contents)?;

    // Option to bypass timer (set to true to start immediately)
    let bypass_timer = &select_config.bypass;
    // Set your desired start time (24-hour format)
    let target_time_str = &select_config.time;

    let concurrency = 10;
    let max_retries = 15;

    if !bypass_timer {
        // Parse the target time
        let target_time = NaiveTime::parse_from_str(target_time_str, "%H:%M:%S")?;

        // Get current time and calculate wait duration
        let now = Local::now();
        let mut target_datetime = now.date_naive().and_time(target_time);

        // If target time has already passed today, schedule for tomorrow
        if target_datetime < now.naive_local() {
            target_datetime = target_datetime + chrono::Duration::days(1);
        }

        let target_local = target_datetime.and_local_timezone(Local).unwrap();

        println!("Scheduled to start at {} (in {:?})",
                 target_local.format("%Y-%m-%d %H:%M:%S"),
                 target_local.signed_duration_since(now));

        // Wait until the scheduled time
        tokio::time::sleep((target_local - now).to_std()?).await;
    } else {
        println!("Timer bypassed, starting immediately");
    }

    println!("Starting execution at {}", Local::now().format("%H:%M:%S"));

    let message_values = select_config.target;

    let wilma2sid = &creds.session_id;
    let formkey = &creds.formkey;
    let school_id = &creds.school_id;

    // Pre-allocate headers to avoid rebuilding them per request
    let mut headers = HeaderMap::new();
    headers.insert("Host", HeaderValue::from_static("ouka.inschool.fi"));
    headers.insert("User-Agent", HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:149.0) Gecko/20100101 Firefox/149.0"));
    headers.insert("Accept", HeaderValue::from_static("*/*"));
    headers.insert("Accept-Language", HeaderValue::from_static("en-GB,en;q=0.9"));
    headers.insert("Accept-Encoding", HeaderValue::from_static("identity"));

    let referer = format!("https://ouka.inschool.fi/!{}/selection/view?", school_id);
    headers.insert("Referer", HeaderValue::from_str(&referer)?);

    headers.insert("Content-Type", HeaderValue::from_static("application/x-www-form-urlencoded"));
    headers.insert("Origin", HeaderValue::from_static("https://ouka.inschool.fi"));
    headers.insert("DNT", HeaderValue::from_static("1"));
    headers.insert("Sec-GPC", HeaderValue::from_static("1"));
    headers.insert("Connection", HeaderValue::from_static("keep-alive"));

    let cookie = format!("enableAnalytics_85932=false; Wilma2SID={}", wilma2sid);
    headers.insert("Cookie", HeaderValue::from_str(&cookie)?);

    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("empty"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("cors"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("same-origin"));
    headers.insert("Pragma", HeaderValue::from_static("no-cache"));
    headers.insert("Cache-Control", HeaderValue::from_static("no-cache"));
    headers.insert("TE", HeaderValue::from_static("trailers"));

    // Create HTTP client with timeout configuration and default headers
    let client = Arc::new(Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(concurrency as usize)
        .default_headers(headers)
        .tcp_nodelay(true) // Optimize TCP to send data directly without delay
        .build()?);

    let url = format!("https://ouka.inschool.fi/!{}/selection/postback", school_id);
    let start = Instant::now();

    let mut retry_attempts = 0;
    let mut pending = message_values.clone();
    let total = pending.len();
    let url_clone = url.clone();

    while !pending.is_empty() && retry_attempts < max_retries {
        if retry_attempts > 0 {
            let backoff = Duration::from_millis(1000);
            println!("Retry attempt {}/{}. Waiting {:?} before retrying {} items...",
                     retry_attempts, max_retries, backoff, pending.len());
            sleep(backoff).await;
        }

        let formkey_clone = formkey.to_string();
        let url_shared = url_clone.clone();

        pending = stream::iter(std::mem::take(&mut pending))
            .map(|msg| {
                let client = Arc::clone(&client);
                let formkey = formkey_clone.clone();
                let url = url_shared.clone();
                async move {
                    match send_request(client, &msg, &formkey, &url).await {
                        Ok(_) => {
                            println!("{} succeeded", msg);
                            (msg, true)
                        },
                        Err(e) => {
                            eprintln!("{} failed: {}", msg, e);
                            (msg, false)
                        }
                    }
                }
            })
            .buffer_unordered(concurrency)
            .filter_map(|(msg, success)| async move {
                if success {
                    None
                } else {
                    Some(msg)
                }
            })
            .collect()
            .await;

        println!("Progress: {}/{} completed ({:.1}%)",
                 total - pending.len(),
                 total,
                 (total - pending.len()) as f64 / total as f64 * 100.0);

        if !pending.is_empty() {
            retry_attempts += 1;
        }
    }

    if pending.is_empty() {
        println!("All tasks completed successfully in {:?}", start.elapsed());
    } else {
        println!("Completed {}/{} tasks in {:?}. {} tasks failed after {} retries.",
                 total - pending.len(), total, start.elapsed(), pending.len(), max_retries);
    }

    Ok(())
}