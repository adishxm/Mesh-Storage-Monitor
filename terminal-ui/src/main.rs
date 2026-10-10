use anyhow::Result;
use serde_json::Value;
use std::env;
use std::time::Duration;
use tokio::time::sleep;

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub fn render_progress_bar(ratio: f64, width: usize) -> String {
    let clamped = ratio.clamp(0.0, 1.0);
    let filled_len = (clamped * width as f64).round() as usize;
    let empty_len = width.saturating_sub(filled_len);

    let filled = "█".repeat(filled_len);
    let empty = "░".repeat(empty_len);

    let color = if clamped > 0.85 {
        "\x1b[31m" // Red warning
    } else if clamped > 0.65 {
        "\x1b[33m" // Yellow caution
    } else {
        "\x1b[32m" // Green healthy
    };

    format!("[{}{}{}\x1b[0m]", color, filled, empty)
}

pub fn render_dashboard(status: &Value, credits: Option<&Value>) -> String {
    let mut out = String::new();

    let peer_id = status["peer_id"].as_str().unwrap_or("N/A");
    let state = status["state"].as_str().unwrap_or("Active");
    let state_badge = match state {
        "Active" => "\x1b[1;42;30m ACTIVE \x1b[0m",
        "Paused" => "\x1b[1;43;30m PAUSED \x1b[0m",
        "Leaving" => "\x1b[1;41;37m LEAVING \x1b[0m",
        _ => "\x1b[1;47;30m UNKNOWN \x1b[0m",
    };

    let used = status["storage_used"].as_u64().unwrap_or(0);
    let quota = status["storage_quota"].as_u64().unwrap_or(0);
    let ratio = if quota == 0 {
        0.0
    } else {
        used as f64 / quota as f64
    };
    let bar = render_progress_bar(ratio, 24);

    let connected_peers = status["peers"].as_array().map(|a| a.len()).unwrap_or(0);
    let trusted_peers = status["trusted_peers"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let nat_status = status["nat_status"].as_str().unwrap_or("Unknown");
    let shards_count = status["shards"].as_array().map(|a| a.len()).unwrap_or(0);

    let bw_limit = match status["bandwidth_limit_kbps"].as_u64() {
        Some(l) => format!("{} KB/s", l),
        None => "Uncapped".to_string(),
    };

    out.push_str("\x1b[1;36m┌────────────────────────────────────────────────────────────────────────┐\x1b[0m\n");
    out.push_str("\x1b[1;36m│          MESH STORAGE MONITOR — LIVE TELEMETRY DASHBOARD               │\x1b[0m\n");
    out.push_str("\x1b[1;36m├────────────────────────────────────────────────────────────────────────┤\x1b[0m\n");
    out.push_str(&format!(
        "│ Node Identity : {:<44} {} │\n",
        peer_id, state_badge
    ));
    out.push_str(&format!(
        "│ NAT Status    : {:<20} Bandwidth Limit: {:<16} │\n",
        nat_status, bw_limit
    ));
    out.push_str(&format!(
        "│ Mesh Peers    : {:<20} Shards Stored  : {:<16} │\n",
        format!("{} live ({} trust)", connected_peers, trusted_peers),
        shards_count
    ));
    out.push_str("\x1b[1;36m├────────────────────────────────────────────────────────────────────────┤\x1b[0m\n");
    out.push_str(&format!(
        "│ Storage Usage : {} {:>6.1}%  ({} / {}) │\n",
        bar,
        ratio * 100.0,
        format_bytes(used),
        format_bytes(quota)
    ));

    if let Some(c) = credits {
        let tier = c["tier"].as_str().unwrap_or("Probationary");
        let tier_color = match tier {
            "Contributor" => "\x1b[1;32m",
            "Probationary" => "\x1b[1;34m",
            "Throttled" => "\x1b[1;33m",
            "Suspended" => "\x1b[1;31m",
            _ => "\x1b[1m",
        };
        let contributed = c["bytes_contributed"].as_u64().unwrap_or(0);
        let consumed = c["bytes_consumed"].as_u64().unwrap_or(0);
        let allowance = c["earned_allowance_bytes"].as_u64().unwrap_or(0);
        let fair_share = c["fair_share_ratio"].as_f64().unwrap_or(1.0);
        let audits_p = c["audits_passed"].as_u64().unwrap_or(0);
        let audits_f = c["audits_failed"].as_u64().unwrap_or(0);

        out.push_str("\x1b[1;36m├────────────────────────────────────────────────────────────────────────┤\x1b[0m\n");
        out.push_str(&format!(
            "│ Reciprocity   : Service Tier: {}{:<14}\x1b[0m Fair Share: {:>5.2}x          │\n",
            tier_color, tier, fair_share
        ));
        out.push_str(&format!(
            "│ Contribution  : Contributed : {:<10} Consumed: {:<18} │\n",
            format_bytes(contributed),
            format_bytes(consumed)
        ));
        out.push_str(&format!(
            "│ Storage Credit: Allowance   : {:<10} Audits  : {:<5} pass / {:<3} fail │\n",
            format_bytes(allowance),
            audits_p,
            audits_f
        ));
    }

    out.push_str("\x1b[1;36m└────────────────────────────────────────────────────────────────────────┘\x1b[0m\n");
    out
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let mut api_url = "http://127.0.0.1:3000".to_string();
    let mut once_mode = false;
    let mut interval_secs = 2u64;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--url" | "-u" => {
                if i + 1 < args.len() {
                    api_url = args[i + 1].clone();
                    i += 1;
                }
            }
            "--interval" | "-i" => {
                if i + 1 < args.len() {
                    if let Ok(v) = args[i + 1].parse() {
                        interval_secs = v;
                    }
                    i += 1;
                }
            }
            "--once" => {
                once_mode = true;
            }
            _ => {}
        }
        i += 1;
    }

    let client = reqwest::Client::new();
    let base_url = api_url.trim_end_matches('/');

    loop {
        let status_url = format!("{}/api/v1/status", base_url);
        let credits_url = format!("{}/api/v1/credits/me", base_url);

        let status_res = client.get(&status_url).send().await;
        match status_res {
            Ok(resp) => {
                if let Ok(status) = resp.json::<Value>().await {
                    let credits = match client.get(&credits_url).send().await {
                        Ok(cr) => cr.json::<Value>().await.ok(),
                        Err(_) => None,
                    };

                    let rendered = render_dashboard(&status, credits.as_ref());
                    if !once_mode {
                        print!("\x1b[2J\x1b[H"); // Clear screen & reset cursor
                    }
                    print!("{}", rendered);
                }
            }
            Err(e) => {
                if !once_mode {
                    print!("\x1b[2J\x1b[H");
                }
                println!(
                    "\x1b[1;31m[!] Cannot connect to mesh node at {}: {}\x1b[0m",
                    base_url, e
                );
                println!("Retrying in {}s...", interval_secs);
            }
        }

        if once_mode {
            break;
        }

        sleep(Duration::from_secs(interval_secs)).await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1024 * 1024 * 10), "10.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 5), "5.00 GB");
    }

    #[test]
    fn test_render_progress_bar() {
        let empty_bar = render_progress_bar(0.0, 10);
        assert!(empty_bar.contains("░░░░░░░░░░"));

        let half_bar = render_progress_bar(0.5, 10);
        assert!(half_bar.contains("█████"));
        assert!(half_bar.contains("░░░░░"));

        let full_bar = render_progress_bar(1.0, 10);
        assert!(full_bar.contains("██████████"));
    }

    #[test]
    fn test_render_dashboard_snapshot() {
        let status = serde_json::json!({
            "peer_id": "12D3KooWTestPeerId1234567890abcdef",
            "state": "Active",
            "storage_used": 500_000_000,
            "storage_quota": 1_000_000_000,
            "usage_ratio": 0.5,
            "peers": ["peer-b", "peer-c"],
            "trusted_peers": ["peer-b"],
            "nat_status": "Public",
            "shards": ["hash1", "hash2", "hash3"],
            "bandwidth_limit_kbps": 1024
        });

        let credits = serde_json::json!({
            "tier": "Contributor",
            "bytes_contributed": 2_000_000_000u64,
            "bytes_consumed": 500_000_000u64,
            "earned_allowance_bytes": 3_000_000_000u64,
            "fair_share_ratio": 4.0,
            "audits_passed": 12,
            "audits_failed": 0
        });

        let output = render_dashboard(&status, Some(&credits));
        assert!(output.contains("12D3KooWTestPeerId1234567890abcdef"));
        assert!(output.contains("ACTIVE"));
        assert!(output.contains("Contributor"));
        assert!(output.contains("4.00x"));
        assert!(output.contains("1024 KB/s"));
    }
}
