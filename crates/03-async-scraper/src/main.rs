use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{sleep, Duration};
use tracing::{info, instrument, warn};

#[derive(Debug, Clone, Copy)]
enum ScrapeType {
    Html,
    Rss,
}

#[derive(Debug)]
struct ScrapeRequest {
    url: String,
    kind: ScrapeType,
}

#[derive(Debug)]
struct ScrapeResult {
    link: String,
    title: String,
    publish_date: String,
    kind: ScrapeType,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    info!("Starting multi-target async-scraper pipeline");

    let (request_tx, request_rx) = mpsc::channel::<ScrapeRequest>(5);
    let (result_tx, mut result_rx) = mpsc::channel::<ScrapeResult>(20);

    let client = Arc::new(
        reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36")
            .timeout(Duration::from_secs(10))
            .build()?,
    );

    let request_rx = Arc::new(Mutex::new(request_rx));

    // Spawn 2 workers
    let mut worker_handles = vec![];
    for id in 0..2 {
        let rx = Arc::clone(&request_rx);
        let tx = result_tx.clone();
        let c = Arc::clone(&client);
        worker_handles.push(tokio::spawn(worker(id, rx, tx, c)));
    }
    drop(result_tx);

    // Producer task
    let producer_handle = tokio::spawn(async move {
        let targets = vec![
            ("https://www.freecodecamp.org/news/", ScrapeType::Html),
            ("https://www.cncf.io/blog/feed/", ScrapeType::Rss),
        ];

        for (url, kind) in targets {
            info!("Producer queuing {} [{:?}]", url, kind);
            if let Err(_) = request_tx.send(ScrapeRequest { url: url.to_string(), kind }).await {
                break;
            }
        }
    });

    // Result consumer
    let consumer_handle = tokio::spawn(async move {
        println!("{:<10} | {:<50} | {:<25} | {}", "KIND", "TITLE", "DATE", "LINK");
        println!("{:-<120}", "");
        while let Some(result) = result_rx.recv().await {
            println!(
                "{:<10} | {:<50} | {:<25} | {}",
                format!("{:?}", result.kind),
                if result.title.len() > 47 { format!("{}...", &result.title[..47]) } else { result.title },
                result.publish_date,
                result.link
            );
        }
    });

    producer_handle.await?;
    for handle in worker_handles {
        handle.await?;
    }
    consumer_handle.await?;

    info!("Scraper pipeline shut down gracefully");
    Ok(())
}

#[instrument(skip(rx, tx, client))]
async fn worker(
    id: usize,
    rx: Arc<Mutex<mpsc::Receiver<ScrapeRequest>>>,
    tx: mpsc::Sender<ScrapeResult>,
    client: Arc<reqwest::Client>,
) {
    info!("Worker {} started", id);
    loop {
        let request = {
            let mut rx_lock = rx.lock().await;
            rx_lock.recv().await
        };

        if let Some(req) = request {
            info!("Worker {} processing [{:?}] {}", id, req.kind, req.url);

            match client.get(&req.url).send().await {
                Ok(response) => {
                    let status = response.status();
                    let bytes = response.bytes().await.unwrap_or_default();
                    info!("Worker {} fetched {} (Status: {}, Bytes: {})", id, req.url, status, bytes.len());

                    match req.kind {
                        ScrapeType::Html => {
                            let results = parse_fcc_news_page(&bytes);
                            for res in results {
                                let _ = tx.send(res).await;
                            }
                        }
                        ScrapeType::Rss => {
                            let results = parse_rss(&bytes);
                            for res in results {
                                if let Err(_) = tx.send(res).await {
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Worker {} failed to fetch {}: {}", id, req.url, e);
                }
            }
            sleep(Duration::from_millis(200)).await;
        } else {
            break;
        }
    }
    info!("Worker {} finished", id);
}

fn parse_fcc_news_page(bytes: &[u8]) -> Vec<ScrapeResult> {
    let html = String::from_utf8_lossy(bytes);
    let document = scraper::Html::parse_document(&html);
    
    let article_selector = scraper::Selector::parse("article").unwrap();
    let title_selector = scraper::Selector::parse("h2").unwrap();
    let link_selector = scraper::Selector::parse("a").unwrap();
    let time_selector = scraper::Selector::parse("time").unwrap();

    document.select(&article_selector)
        .take(5)
        .map(|article| {
            let title = article.select(&title_selector)
                .next()
                .map(|el| el.text().collect::<String>())
                .unwrap_or_else(|| "No Title".to_string());

            let link = article.select(&link_selector)
                .next()
                .and_then(|el| el.value().attr("href"))
                .map(|href| {
                    if href.starts_with("http") {
                        href.to_string()
                    } else {
                        format!("https://www.freecodecamp.org{}", href)
                    }
                })
                .unwrap_or_default();

            let publish_date = article.select(&time_selector)
                .next()
                .and_then(|el| el.value().attr("datetime"))
                .unwrap_or("Unknown Date")
                .to_string();

            ScrapeResult {
                link,
                title: title.trim().to_string(),
                publish_date,
                kind: ScrapeType::Html,
            }
        })
        .collect()
}

fn parse_rss(bytes: &[u8]) -> Vec<ScrapeResult> {
    match feed_rs::parser::parse(bytes) {
        Ok(feed) => {
            feed.entries
                .into_iter()
                .take(5)
                .map(|entry| {
                    let link = entry.links.first().map(|l| l.href.clone()).unwrap_or_default();
                    let title = entry.title.map(|t| t.content).unwrap_or_default();
                    let publish_date = entry.published
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_else(|| "Unknown Date".to_string());
                    
                    ScrapeResult {
                        link,
                        title,
                        publish_date,
                        kind: ScrapeType::Rss,
                    }
                })
                .collect()
        }
        Err(_) => vec![],
    }
}
