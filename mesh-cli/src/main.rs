use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use reqwest::multipart;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "mesh-cli",
    version = "0.1.0",
    about = "Production CLI client for Mesh Storage Monitor peer-to-peer cloud"
)]
struct Cli {
    #[arg(
        short,
        long,
        default_value = "http://127.0.0.1:3000",
        help = "Local node REST API endpoint URL"
    )]
    url: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(about = "Display node telemetry, peers, quota, NAT, and reciprocity tier")]
    Status,

    #[command(about = "List connected and trusted peers with reliability and credit status")]
    Peers,

    #[command(about = "Manage invitation tokens and enrollment")]
    Invite {
        #[command(subcommand)]
        action: InviteAction,
    },

    #[command(about = "Encrypt, chunk, and distribute a local file across the mesh")]
    Upload {
        #[arg(help = "Path to file to upload")]
        file_path: PathBuf,
    },

    #[command(about = "Retrieve, reconstruct, and decrypt a file from the mesh")]
    Download {
        #[arg(help = "Target File ID to download")]
        file_id: String,

        #[arg(short, long, help = "Decryption passphrase")]
        passphrase: Option<String>,

        #[arg(short, long, help = "Hex salt")]
        salt: Option<String>,

        #[arg(short, long, default_value_t = 2, help = "Data shards (k)")]
        k: usize,

        #[arg(short, long, default_value_t = 1, help = "Parity shards (m)")]
        m: usize,

        #[arg(short, long, help = "Output file destination path")]
        out: Option<PathBuf>,
    },

    #[command(about = "Inspect or modify node storage quota allocation")]
    Quota {
        #[arg(short = 'g', long, help = "Storage quota in gigabytes (e.g. 2.5)")]
        gb: Option<f64>,

        #[arg(short = 'b', long, help = "Storage quota in exact bytes")]
        bytes: Option<u64>,
    },

    #[command(about = "Inspect or configure node bandwidth rate limiter")]
    Bandwidth {
        #[arg(
            short = 'l',
            long,
            help = "Egress rate limit in KB/s (0 or omit to disable)"
        )]
        limit: Option<u64>,
    },

    #[command(about = "Display reciprocity credit ledger, earned allowance, and service tier")]
    Credits,

    #[command(about = "Inspect chunk degradation and repair feasibility for a manifest")]
    RepairCheck {
        #[arg(help = "File ID to audit")]
        file_id: String,
    },

    #[command(about = "Temporarily pause node storage participation (graceful sleep)")]
    Pause,

    #[command(about = "Resume node storage participation after pause")]
    Resume,

    #[command(about = "Initiate permanent leave and trigger shard relocation")]
    Leave,

    #[command(about = "Disaster recovery backup export and restoration")]
    Backup {
        #[command(subcommand)]
        action: BackupAction,
    },

    #[command(about = "Display Prometheus observability metrics")]
    Metrics,
}

#[derive(Subcommand, Debug)]
enum BackupAction {
    #[command(about = "Create an encrypted disaster recovery backup archive")]
    Export {
        #[arg(short, long, help = "Passphrase to encrypt the backup")]
        passphrase: Option<String>,

        #[arg(short, long, help = "Output destination file path for .mbak archive")]
        out: Option<PathBuf>,
    },

    #[command(about = "Restore node state from an encrypted backup archive")]
    Restore {
        #[arg(help = "Path to the .mbak backup archive file")]
        file_path: PathBuf,

        #[arg(short, long, help = "Passphrase to decrypt the backup")]
        passphrase: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum InviteAction {
    #[command(about = "Create a single-use signed invitation for a new device")]
    Create {
        #[arg(
            short,
            long,
            help = "Invitation validity duration in seconds (default 86400)"
        )]
        duration: Option<u64>,
    },

    #[command(about = "Join network using an invitation QR payload or token")]
    Join {
        #[arg(help = "Invitation QR payload or JSON string")]
        token: String,
    },
}

fn format_bytes(bytes: u64) -> String {
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = reqwest::Client::new();
    let base_url = cli.url.trim_end_matches('/');

    match cli.command {
        Commands::Status => {
            let status_url = format!("{}/api/v1/status", base_url);
            let credits_url = format!("{}/api/v1/credits/me", base_url);

            let status_resp: Value = client
                .get(&status_url)
                .send()
                .await
                .context("Failed to connect to mesh node. Is it running?")?
                .json()
                .await?;

            let credits_resp: Option<Value> = match client.get(&credits_url).send().await {
                Ok(r) => r.json().await.ok(),
                Err(_) => None,
            };

            println!("\x1b[1;36m====================================================\x1b[0m");
            println!("\x1b[1;36m           MESH STORAGE MONITOR — NODE STATUS       \x1b[0m");
            println!("\x1b[1;36m====================================================\x1b[0m");
            println!(
                " Peer ID       : \x1b[1m{}\x1b[0m",
                status_resp["peer_id"].as_str().unwrap_or("N/A")
            );

            let state = status_resp["state"].as_str().unwrap_or("Active");
            let state_color = match state {
                "Active" => "\x1b[1;32m",
                "Paused" => "\x1b[1;33m",
                "Leaving" | "Revoked" => "\x1b[1;31m",
                _ => "\x1b[1m",
            };
            println!(" State         : {}{}\x1b[0m", state_color, state);

            let used = status_resp["storage_used"].as_u64().unwrap_or(0);
            let quota = status_resp["storage_quota"].as_u64().unwrap_or(0);
            let ratio = status_resp["usage_ratio"].as_f64().unwrap_or(0.0) * 100.0;
            println!(
                " Storage Quota : {} / {} ({:.1}%)",
                format_bytes(used),
                format_bytes(quota),
                ratio
            );

            let peers = status_resp["peers"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            let trusted = status_resp["trusted_peers"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0);
            println!(" Mesh Peers    : {} connected ({} trusted)", peers, trusted);

            let nat = status_resp["nat_status"].as_str().unwrap_or("Unknown");
            println!(" NAT Status    : {}", nat);

            if let Some(relays) = status_resp["relay_addresses"].as_array()
                && !relays.is_empty()
            {
                println!(" Relay Addrs   : {} active", relays.len());
            }

            if let Some(credits) = credits_resp {
                println!("\x1b[1;34m----------------------------------------------------\x1b[0m");
                let tier = credits["tier"].as_str().unwrap_or("Probationary");
                let tier_color = match tier {
                    "Contributor" => "\x1b[1;32m",
                    "Probationary" => "\x1b[1;34m",
                    "Throttled" => "\x1b[1;33m",
                    "Suspended" => "\x1b[1;31m",
                    _ => "\x1b[1m",
                };
                println!(" Service Tier  : {}{}\x1b[0m", tier_color, tier);
                let contributed = credits["bytes_contributed"].as_u64().unwrap_or(0);
                let consumed = credits["bytes_consumed"].as_u64().unwrap_or(0);
                let allowance = credits["earned_allowance_bytes"].as_u64().unwrap_or(0);
                println!(
                    " Reciprocity   : Contributed: {} | Consumed: {}",
                    format_bytes(contributed),
                    format_bytes(consumed)
                );
                println!(" Allowance     : {} earned", format_bytes(allowance));
            }
            println!("\x1b[1;36m====================================================\x1b[0m");
        }

        Commands::Peers => {
            let url = format!("{}/api/v1/peers", base_url);
            let resp: Value = client.get(&url).send().await?.json().await?;

            println!("\x1b[1;35m--- Connected Peers ---\x1b[0m");
            if let Some(peers) = resp["peers"].as_array() {
                if peers.is_empty() {
                    println!(" No remote peers currently connected.");
                } else {
                    for p in peers {
                        let p_str = p.as_str().unwrap_or("");
                        let rel_url = format!("{}/api/v1/reliability/{}", base_url, p_str);
                        let rel_info = match client.get(&rel_url).send().await {
                            Ok(r) => r.json::<Value>().await.ok(),
                            Err(_) => None,
                        };
                        let score = rel_info
                            .as_ref()
                            .and_then(|v| v["score"].as_f64())
                            .unwrap_or(1.0);
                        let healthy = rel_info
                            .as_ref()
                            .and_then(|v| v["is_healthy"].as_bool())
                            .unwrap_or(true);
                        let health_badge = if healthy {
                            "\x1b[32m[HEALTHY]\x1b[0m"
                        } else {
                            "\x1b[31m[UNHEALTHY]\x1b[0m"
                        };
                        println!(" - {} (Score: {:.2}) {}", p_str, score, health_badge);
                    }
                }
            }

            if let Some(trusted) = resp["trusted_peers"].as_array() {
                println!("\n\x1b[1;32m--- Trusted Peers (Authorized) ---\x1b[0m");
                for t in trusted {
                    println!(" - {}", t.as_str().unwrap_or(""));
                }
            }
        }

        Commands::Invite { action } => match action {
            InviteAction::Create { duration } => {
                let url = format!("{}/api/v1/invite/create", base_url);
                let payload = serde_json::json!({
                    "duration_secs": duration.unwrap_or(86400)
                });
                let resp: Value = client
                    .post(&url)
                    .json(&payload)
                    .send()
                    .await?
                    .json()
                    .await?;
                println!("\x1b[1;32mInvitation Created Successfully!\x1b[0m");
                println!(
                    "Issuer Peer ID: {}",
                    resp["invitation"]["issuer_peer_id"].as_str().unwrap_or("")
                );
                println!(
                    "Expires At    : {}",
                    resp["invitation"]["expires_at"].as_u64().unwrap_or(0)
                );
                println!("\n\x1b[1;33m--- QR Code / Join Payload ---\x1b[0m");
                println!("{}", resp["qr_payload"].as_str().unwrap_or(""));
            }
            InviteAction::Join { token } => {
                let url = format!("{}/api/v1/invite/join", base_url);
                let payload = serde_json::json!({
                    "qr_payload": token
                });
                let resp: Value = client
                    .post(&url)
                    .json(&payload)
                    .send()
                    .await?
                    .json()
                    .await?;
                if resp["success"].as_bool().unwrap_or(false) {
                    println!("\x1b[1;32mSuccessfully Enrolled into Cluster!\x1b[0m");
                    println!(
                        "Issuer Node: {}",
                        resp["issuer_peer_id"].as_str().unwrap_or("")
                    );
                } else {
                    println!(
                        "\x1b[1;31mEnrollment Failed: {}\x1b[0m",
                        resp["message"].as_str().unwrap_or("Unknown error")
                    );
                }
            }
        },

        Commands::Upload { file_path } => {
            let filename = file_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let data = fs::read(&file_path)
                .with_context(|| format!("Failed to read file: {:?}", file_path))?;
            let len = data.len();

            let part = multipart::Part::bytes(data).file_name(filename.clone());
            let form = multipart::Form::new().part("file", part);

            let url = format!("{}/api/v1/upload", base_url);
            println!(
                "Uploading {} ({}) to mesh storage...",
                filename,
                format_bytes(len as u64)
            );

            let resp: Value = client
                .post(&url)
                .multipart(form)
                .send()
                .await?
                .json()
                .await?;

            if resp["status"] == "ok" {
                println!("\x1b[1;32mUpload Succeeded!\x1b[0m");
                println!(
                    "File ID    : \x1b[1m{}\x1b[0m",
                    resp["file_id"].as_str().unwrap_or("")
                );
                println!("Root Hash  : {}", resp["root_hash"].as_str().unwrap_or(""));
                println!("Chunks     : {}", resp["chunks"].as_u64().unwrap_or(0));
                println!(
                    "Passphrase : \x1b[1;33m{}\x1b[0m",
                    resp["passphrase"].as_str().unwrap_or("")
                );
                println!("Salt (hex) : {}", resp["salt"].as_str().unwrap_or(""));
                println!(
                    "\x1b[1;31mIMPORTANT:\x1b[0m Save your Passphrase & Salt! The mesh is zero-knowledge; without them, files cannot be recovered."
                );
            } else {
                println!("\x1b[1;31mUpload Failed: {:?}\x1b[0m", resp);
            }
        }

        Commands::Download {
            file_id,
            passphrase,
            salt,
            k,
            m,
            out,
        } => {
            let url = format!("{}/api/v1/download/{}", base_url, file_id);
            let query = [
                (
                    "passphrase",
                    passphrase.unwrap_or_else(|| "cluster-secret-passphrase-2026".to_string()),
                ),
                ("salt", salt.unwrap_or_else(|| hex::encode([0u8; 16]))),
                ("k", k.to_string()),
                ("m", m.to_string()),
            ];

            println!("Retrieving shards for {}...", file_id);
            let resp = client.get(&url).query(&query).send().await?;

            if !resp.status().is_success() {
                let err_text = resp.text().await.unwrap_or_default();
                eprintln!("\x1b[1;31mDownload failed: {}\x1b[0m", err_text);
                return Ok(());
            }

            let file_bytes = resp.bytes().await?;
            let dest_path = out.unwrap_or_else(|| PathBuf::from(format!("downloaded_{}", file_id)));
            fs::write(&dest_path, &file_bytes)?;

            println!("\x1b[1;32mReconstruction Complete!\x1b[0m");
            println!(
                "Saved {} to {:?}",
                format_bytes(file_bytes.len() as u64),
                dest_path
            );
        }

        Commands::Quota { gb, bytes } => {
            let url = format!("{}/api/v1/quota", base_url);
            if gb.is_some() || bytes.is_some() {
                let payload = serde_json::json!({
                    "quota_gb": gb,
                    "quota_bytes": bytes,
                });
                let resp: Value = client
                    .post(&url)
                    .json(&payload)
                    .send()
                    .await?
                    .json()
                    .await?;
                println!("\x1b[1;32mQuota Updated!\x1b[0m");
                let quota = resp["storage_quota"].as_u64().unwrap_or(0);
                println!("New Quota: {}", format_bytes(quota));
            } else {
                let resp: Value = client.get(&url).send().await?.json().await?;
                let used = resp["storage_used"].as_u64().unwrap_or(0);
                let quota = resp["storage_quota"].as_u64().unwrap_or(0);
                let ratio = resp["usage_ratio"].as_f64().unwrap_or(0.0) * 100.0;
                println!(
                    "Quota Allocation: {} / {} ({:.1}%)",
                    format_bytes(used),
                    format_bytes(quota),
                    ratio
                );
                println!(
                    "Remaining       : {}",
                    format_bytes(resp["remaining_bytes"].as_u64().unwrap_or(0))
                );
            }
        }

        Commands::Bandwidth { limit } => {
            let url = format!("{}/api/v1/bandwidth", base_url);
            if let Some(lim) = limit {
                let lim_val = if lim == 0 { None } else { Some(lim) };
                let payload = serde_json::json!({
                    "limit_kbps": lim_val,
                });
                let resp: Value = client
                    .post(&url)
                    .json(&payload)
                    .send()
                    .await?
                    .json()
                    .await?;
                if resp["success"].as_bool().unwrap_or(false) {
                    println!(
                        "\x1b[1;32mBandwidth Limit Updated: {:?} KB/s\x1b[0m",
                        lim_val
                    );
                }
            } else {
                let resp: Value = client.get(&url).send().await?.json().await?;
                match resp["limit_kbps"].as_u64() {
                    Some(k) => println!("Current Bandwidth Limit: {} KB/s", k),
                    None => println!("Bandwidth Limit: Unlimited (Uncapped)"),
                }
            }
        }

        Commands::Credits => {
            let url = format!("{}/api/v1/credits/me", base_url);
            let resp: Value = client.get(&url).send().await?.json().await?;

            println!("\x1b[1;34m====================================================\x1b[0m");
            println!("\x1b[1;34m             RECIPROCITY CREDIT LEDGER              \x1b[0m");
            println!("\x1b[1;34m====================================================\x1b[0m");
            println!(
                " Peer ID          : {}",
                resp["peer_id"].as_str().unwrap_or("")
            );
            println!(
                " Service Tier     : \x1b[1;32m{}\x1b[0m",
                resp["tier"].as_str().unwrap_or("")
            );
            println!(
                " Contributed      : {}",
                format_bytes(resp["bytes_contributed"].as_u64().unwrap_or(0))
            );
            println!(
                " Consumed         : {}",
                format_bytes(resp["bytes_consumed"].as_u64().unwrap_or(0))
            );
            println!(
                " Net Balance      : {}",
                resp["credit_balance"].as_i64().unwrap_or(0)
            );
            println!(
                " Earned Allowance : {}",
                format_bytes(resp["earned_allowance_bytes"].as_u64().unwrap_or(0))
            );
            println!(
                " Fair Share Ratio : {:.2}x",
                resp["fair_share_ratio"].as_f64().unwrap_or(1.0)
            );
            println!(
                " Audits Passed    : {}",
                resp["audits_passed"].as_u64().unwrap_or(0)
            );
            println!(
                " Audits Failed    : {}",
                resp["audits_failed"].as_u64().unwrap_or(0)
            );
            println!(
                " Uptime (seconds) : {}",
                resp["uptime_seconds"].as_u64().unwrap_or(0)
            );
            println!("\x1b[1;34m====================================================\x1b[0m");
        }

        Commands::RepairCheck { file_id } => {
            let url = format!("{}/api/v1/repair/check/{}", base_url, file_id);
            let resp: Value = client.get(&url).send().await?.json().await?;

            println!("\x1b[1;33m--- Shard Redundancy & Repair Inspection ---\x1b[0m");
            println!(
                "File ID        : {}",
                resp["file_id"].as_str().unwrap_or("")
            );
            let can_repair = resp["can_repair_all"].as_bool().unwrap_or(false);
            let badge = if can_repair {
                "\x1b[32m[RECOVERABLE]\x1b[0m"
            } else {
                "\x1b[31m[DEGRADED/AT RISK]\x1b[0m"
            };
            println!("Repair Status  : {}", badge);

            if let Some(chunks) = resp["degraded_chunks"].as_array() {
                if chunks.is_empty() {
                    println!("\x1b[32mAll chunks healthy! Redundancy floor intact.\x1b[0m");
                } else {
                    println!("Degraded Chunks ({} require repair):", chunks.len());
                    for c in chunks {
                        println!(
                            " - Chunk #{}: Missing Shard Indices: {:?}, Surviving: {:?}",
                            c["chunk_idx"].as_u64().unwrap_or(0),
                            c["missing_indices"],
                            c["surviving_indices"]
                        );
                    }
                }
            }
        }

        Commands::Pause => {
            let url = format!("{}/api/v1/pause", base_url);
            let resp: Value = client.post(&url).send().await?.json().await?;
            println!("Status: {}", resp["state"].as_str().unwrap_or(""));
            println!("{}", resp["message"].as_str().unwrap_or("Node paused."));
        }

        Commands::Resume => {
            let url = format!("{}/api/v1/resume", base_url);
            let resp: Value = client.post(&url).send().await?.json().await?;
            println!("Status: {}", resp["state"].as_str().unwrap_or(""));
            println!("{}", resp["message"].as_str().unwrap_or("Node resumed."));
        }

        Commands::Leave => {
            let url = format!("{}/api/v1/leave", base_url);
            let resp: Value = client.post(&url).send().await?.json().await?;
            println!("Status: {}", resp["state"].as_str().unwrap_or(""));
            println!("{}", resp["message"].as_str().unwrap_or("Leave initiated."));
        }

        Commands::Backup { action } => match action {
            BackupAction::Export { passphrase, out } => {
                let pass = passphrase.unwrap_or_else(|| "default_mesh_backup_key".to_string());
                let url = format!("{}/api/v1/backup/export", base_url);
                let resp: Value = client
                    .post(&url)
                    .json(&serde_json::json!({ "passphrase": pass }))
                    .send()
                    .await?
                    .json()
                    .await?;

                let hex_str = resp["archive_hex"]
                    .as_str()
                    .context("Missing archive_hex")?;
                let bytes = hex::decode(hex_str).context("Failed to decode archive hex")?;
                let dest = out.unwrap_or_else(|| PathBuf::from("node_backup.mbak"));
                fs::write(&dest, &bytes)?;
                println!("\x1b[1;32m✓ Backup exported successfully!\x1b[0m");
                println!(
                    " Archive Size : {} ({} bytes)",
                    format_bytes(bytes.len() as u64),
                    bytes.len()
                );
                println!(" Saved To     : {}", dest.display());
            }
            BackupAction::Restore {
                file_path,
                passphrase,
            } => {
                let pass = passphrase.unwrap_or_else(|| "default_mesh_backup_key".to_string());
                let bytes = fs::read(&file_path).with_context(|| {
                    format!("Failed to read backup file at {}", file_path.display())
                })?;
                let hex_str = hex::encode(bytes);

                let url = format!("{}/api/v1/backup/restore", base_url);
                let resp: Value = client
                    .post(&url)
                    .json(&serde_json::json!({
                        "passphrase": pass,
                        "archive_hex": hex_str
                    }))
                    .send()
                    .await?
                    .json()
                    .await?;

                println!("\x1b[1;32m✓ Node state restored from disaster recovery backup!\x1b[0m");
                println!(
                    " Restored Manifests : {}",
                    resp["restored_manifests"].as_u64().unwrap_or(0)
                );
                println!(
                    " Restored Peers     : {}",
                    resp["restored_peers"].as_u64().unwrap_or(0)
                );
                println!(
                    " Storage Quota      : {}",
                    format_bytes(resp["storage_quota"].as_u64().unwrap_or(0))
                );
            }
        },

        Commands::Metrics => {
            let url = format!("{}/api/v1/metrics", base_url);
            let resp = client.get(&url).send().await?.text().await?;
            println!("{}", resp);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1024 * 1024 * 5), "5.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 2), "2.00 GB");
    }

    #[test]
    fn test_cli_parse_status() {
        let cli = Cli::try_parse_from(["mesh-cli", "status"]).unwrap();
        assert_eq!(cli.url, "http://127.0.0.1:3000");
        match cli.command {
            Commands::Status => {}
            _ => panic!("Expected Status command"),
        }
    }

    #[test]
    fn test_cli_parse_quota_and_url() {
        let cli = Cli::try_parse_from([
            "mesh-cli",
            "--url",
            "http://node.local:4000",
            "quota",
            "--gb",
            "3.5",
        ])
        .unwrap();
        assert_eq!(cli.url, "http://node.local:4000");
        match cli.command {
            Commands::Quota { gb, bytes } => {
                assert_eq!(gb, Some(3.5));
                assert_eq!(bytes, None);
            }
            _ => panic!("Expected Quota command"),
        }
    }

    #[test]
    fn test_cli_parse_download_parameters() {
        let cli = Cli::try_parse_from([
            "mesh-cli",
            "download",
            "f-test-123",
            "--passphrase",
            "secret",
            "-k",
            "3",
            "-m",
            "2",
        ])
        .unwrap();
        match cli.command {
            Commands::Download {
                file_id,
                passphrase,
                k,
                m,
                ..
            } => {
                assert_eq!(file_id, "f-test-123");
                assert_eq!(passphrase.as_deref(), Some("secret"));
                assert_eq!(k, 3);
                assert_eq!(m, 2);
            }
            _ => panic!("Expected Download command"),
        }
    }

    #[test]
    fn test_cli_parse_credits_and_repair() {
        let cli = Cli::try_parse_from(["mesh-cli", "credits"]).unwrap();
        match cli.command {
            Commands::Credits => {}
            _ => panic!("Expected Credits command"),
        }

        let cli_repair = Cli::try_parse_from(["mesh-cli", "repair-check", "file_abc"]).unwrap();
        match cli_repair.command {
            Commands::RepairCheck { file_id } => {
                assert_eq!(file_id, "file_abc");
            }
            _ => panic!("Expected RepairCheck command"),
        }
    }

    #[test]
    fn test_cli_parse_backup_and_metrics() {
        let cli_export = Cli::try_parse_from([
            "mesh-cli",
            "backup",
            "export",
            "--passphrase",
            "mypass",
            "--out",
            "archive.mbak",
        ])
        .unwrap();
        match cli_export.command {
            Commands::Backup { action } => match action {
                BackupAction::Export { passphrase, out } => {
                    assert_eq!(passphrase.as_deref(), Some("mypass"));
                    assert_eq!(out, Some(PathBuf::from("archive.mbak")));
                }
                _ => panic!("Expected Export action"),
            },
            _ => panic!("Expected Backup command"),
        }

        let cli_restore = Cli::try_parse_from([
            "mesh-cli",
            "backup",
            "restore",
            "archive.mbak",
            "--passphrase",
            "mypass",
        ])
        .unwrap();
        match cli_restore.command {
            Commands::Backup { action } => match action {
                BackupAction::Restore {
                    file_path,
                    passphrase,
                } => {
                    assert_eq!(file_path, PathBuf::from("archive.mbak"));
                    assert_eq!(passphrase.as_deref(), Some("mypass"));
                }
                _ => panic!("Expected Restore action"),
            },
            _ => panic!("Expected Backup command"),
        }

        let cli_metrics = Cli::try_parse_from(["mesh-cli", "metrics"]).unwrap();
        match cli_metrics.command {
            Commands::Metrics => {}
            _ => panic!("Expected Metrics command"),
        }
    }
}
