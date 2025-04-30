use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};
use chrono::{Local, NaiveTime};
use futures::stream::{self, StreamExt};
use reqwest::Client;
use tokio::time::sleep;

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
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:137.0) Gecko/20100101 Firefox/137.0")
        .header("Accept", "*/*")
        .header("Accept-Language", "en-US,en\\=0.5")
        .header("Accept-Encoding", "gzip, deflate, br, zstd")
        .header("Referer", "https://ouka.inschool.fi/!02227756/selection/view?")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Origin", "https://ouka.inschool.fi")
        .header("DNT", "1")
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
    if body.contains("SrvError") {
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
    // Option to bypass timer (set to true to start immediately)
    let bypass_timer = false;

    // Set your desired start time (24-hour format)
    let target_time_str = "12:00";

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

    let message_values = vec![
        "56A65493_27319_46162&",
        "56A65493_27319_38845&",
        "56A65493_27319_47276&",
        "56A65493_27319_37304&",
        "56A65493_27319_39950&",
        "56A65493_27319_57503&",
        "56A65493_27319_38968&",
        "56A65493_27548_39346&",
        "56A65493_27548_57468&",
        "56A65493_27548_39280&",
        "56A65493_27548_52473&",
        "56A65493_27548_51813&",
        "56A65493_27548_39322&",
        "56A65493_27548_39362&",
        "56A65493_27623_57478&",
        "56A65493_27623_46945&",
        "56A65493_27623_38999&",
        "56A65493_27623_39695&",
        "56A65493_27623_52454&",
        "56A65493_27623_46049&",
        "56A65493_27677_57488&",
        "56A65493_27677_39291&",
        "56A65493_27677_39154&",
        "56A65493_27677_46898&",
        "56A65493_27677_39307&",
        "56A65493_27677_39941&",
        "56A65493_27682_31709&",
        "56A65493_27682_57491&",
        "56A65493_27682_39299&",
        "56A65493_27682_39313&",
        "56A65493_27682_46893&",
        "56A65493_27682_39358&"
    ];

    // Create HTTP client with timeout configuration
    let client = Arc::new(Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(10)
        .build()?);

    let wilma2sid = "e612928fa4bc102bd2d3faa5f61e99f8";
    let formkey = "student%3A227756%3A429aeb5e611783c7cbad772f36f2133c";
    let start = Instant::now();
    let concurrency = 10;
    let max_retries = 15;

    let mut retry_attempts = 0;
    let mut pending = message_values.clone();
    let total = pending.len();

    while !pending.is_empty() && retry_attempts < max_retries {
        if retry_attempts > 0 {
            let backoff = Duration::from_millis(500 * 2_u64.pow(retry_attempts as u32));
            println!("Retry attempt {}/{}. Waiting {:?} before retrying {} items...",
                     retry_attempts, max_retries, backoff, pending.len());
            sleep(backoff).await;
        }

        pending = stream::iter(std::mem::take(&mut pending))
            .map(|msg| {
                let client = Arc::clone(&client);
                async move {
                    match send_request(client, &msg, wilma2sid, formkey).await {
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