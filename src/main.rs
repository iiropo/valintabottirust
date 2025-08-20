use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};
use chrono::{Local, NaiveTime};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use tokio::time::sleep;

#[derive(Deserialize)]
struct Credentials {
    wilma2sid: String,
    formkey: String,
    time: String,
    bypass: bool,
    target: Vec<String>,
}

pub async fn send_request(
    client: Arc<Client>,
    message_value: &str,
    wilma2sid_value: &str,
    formkey: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let custom_body = format!(
        "message=pick-group&target={}&formkey={}&interest=56A65493_27548&refresh=21810&extras=56A65493",
        message_value, formkey
    );

    let response = client
        .post("https://ouka.inschool.fi/!02227756/selection/postback")
        .header("Host", "ouka.inschool.fi")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:138.0) Gecko/20100101 Firefox/138.0")
        .header("Accept", "*/*")
        .header("Accept-Language", "en-US,en\\=0.5")
        .header("Referer", "https://ouka.inschool.fi/!02227756/selection/view?")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Origin", "https://ouka.inschool.fi")
        .header("Connection", "keep-alive")
        .header("Cookie", format!("Wilma2SID={}", wilma2sid_value))
        .header("Sec-Fetch-Dest", "empty")
        .header("Sec-Fetch-Mode", "cors")
        .header("Sec-Fetch-Site", "same\\-origin")
        .header("TE", "trailers")
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

    // Option to bypass timer (set to true to start immediately)
    let bypass_timer = &creds.bypass;
    // Set your desired start time (24-hour format)
    let target_time_str = &creds.time;

    if !bypass_timer {
        // Parse the target time
        let target_time = NaiveTime::parse_from_str(target_time_str, "%H:%M")?;

        // Get current time and calculate wait duration
        let now = Local::now();
        let mut target_datetime = now.date().and_time(target_time).unwrap();

        // If target time has already passed today, schedule for tomorrow
        if target_datetime < now {
            target_datetime = target_datetime + chrono::Duration::days(1);
        }

        println!("Scheduled to start at {} (in {:?})",
                 target_datetime.format("%Y-%m-%d %H:%M:%S"),
                 target_datetime.signed_duration_since(now));

        // Wait until the scheduled time
        tokio::time::sleep((target_datetime - now).to_std()?).await;
    } else {
        println!("Timer bypassed, starting immediately");
    }

    println!("Starting execution at {}", Local::now().format("%H:%M:%S"));

    let message_values = creds.target;

    // Create HTTP client with timeout configuration
    let client = Arc::new(Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .build()?);

    let wilma2sid = &creds.wilma2sid;
    let formkey = &creds.formkey;
    let start = Instant::now();
    let concurrency = 10;
    let max_retries = 15;

    let mut retry_attempts = 0;
    let mut pending = message_values.clone();
    let total = pending.len();

    while !pending.is_empty() && retry_attempts < max_retries {
        if retry_attempts > 0 {
            let backoff = Duration::from_millis(1000);
            println!("Retry attempt {}/{}. Waiting {:?} before retrying {} items...",
                     retry_attempts, max_retries, backoff, pending.len());
            sleep(backoff).await;
        }

        let wilma2sid_clone = wilma2sid.to_string();
        let formkey_clone = formkey.to_string();

        pending = stream::iter(std::mem::take(&mut pending))
            .map(|msg| {
                let client = Arc::clone(&client);
                let wilma2sid = wilma2sid_clone.clone();
                let formkey = formkey_clone.clone();
                async move {
                    match send_request(client, &msg, &wilma2sid, &formkey).await {
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