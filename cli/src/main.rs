use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use tracing::{info, Level};
use tracing_subscriber::{fmt, EnvFilter};

use backend::{Backend, BypassProxy, ProxyConfig};
use control::{ControlClient, ControlServer, ServerConfig};
use engine::{BypassConfig, Config};

#[derive(Parser)]
#[command(name = "turkeydpi")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[arg(long, default_value = "info")]
    log_level: String,

    #[arg(long)]
    json_logs: bool,

    #[arg(long, default_value = "/tmp/turkeydpi.sock")]
    socket: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Bypass {
        #[arg(short, long, default_value = "127.0.0.1:8844")]
        listen: String,

        #[arg(short, long, default_value = "aggressive")]
        preset: IspPreset,

        #[arg(short, long)]
        verbose: bool,
    },

    Run {
        #[arg(long)]
        proxy: bool,

        #[arg(long, default_value = "127.0.0.1:1080")]
        listen: String,
    },

    Start,
    Stop,
    Status,
    Health,
    Stats,
    ResetStats,
    Validate {
        #[arg(value_name = "FILE")]
        config: PathBuf,
    },
    Reload {
        #[arg(value_name = "FILE")]
        config: PathBuf,
    },
    GenConfig {
        #[arg(long, default_value = "toml")]
        format: String,

        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

fn setup_logging(level: &str, json: bool) -> Result<()> {
    let level = level.parse::<Level>().unwrap_or(Level::INFO);
    let filter = EnvFilter::from_default_env()
        .add_directive(level.into());

    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(false)
        .with_file(true)
        .with_line_number(true);

    if json {
        subscriber.json().init();
    } else {
        subscriber.init();
    }

    Ok(())
}

async fn run_daemon(cli: &Cli, proxy: bool, listen: &str) -> Result<()> {
    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Starting TurkeyDPI engine"
    );

    let config = if let Some(ref path) = cli.config {
        Config::load_from_file(path)
            .with_context(|| format!("Failed to load config from {}", path.display()))?
    } else {
        Config::default()
    };

    info!("Configuration loaded successfully");

    let server_config = ServerConfig {
        socket_path: cli.socket.clone(),
        ..Default::default()
    };

    let mut server = ControlServer::new(server_config, config.clone());
    server.start().await?;

    info!(socket = %cli.socket.display(), "Control server started");

    if proxy {
        info!(listen = %listen, "Starting proxy backend");
        
        let listen_addr: std::net::SocketAddr = listen.parse()
            .with_context(|| format!("Invalid listen address: {}", listen))?;

        let backend_config = backend::BackendConfig {
            engine_config: config,
            max_queue_size: 1000,
            backend_settings: backend::BackendSettings::Proxy(backend::ProxySettings {
                listen_addr,
                ..Default::default()
            }),
        };

        let mut backend = backend::ProxyBackend::new();
        let handle = backend.start(backend_config).await?;

        info!(addr = %listen_addr, "Proxy backend started");

        tokio::signal::ctrl_c().await?;
        info!("Received shutdown signal");

        
        handle.shutdown().await?;
        backend.stop().await?;
    } else {
        info!("Running in control-only mode (use --proxy to start proxy backend)");
        
        tokio::signal::ctrl_c().await?;
        info!("Received shutdown signal");
    }

    server.stop().await?;
    info!("Shutdown complete");

    Ok(())
}

async fn send_command<F, T>(socket: &PathBuf, action: F) -> Result<T>
where
    F: FnOnce(&mut ControlClient) -> std::pin::Pin<Box<dyn std::future::Future<Output = control::Result<T>> + Send + '_>>,
{
    let mut client = ControlClient::new(socket);
    action(&mut client).await
        .with_context(|| format!("Failed to connect to {}", socket.display()))
}

#[derive(Debug, Clone, ValueEnum)]
enum IspPreset {
    /// TT - s @ 2 bit
    TurkTelekom,
    /// VODOFONE - s @ 3 bit
    Vodafone,
    /// SUPONLINE - s @ 4 bit
    Superonline,
    /// AGG
    Aggressive,
}

impl IspPreset {
    fn to_bypass_config(&self) -> BypassConfig {
        match self {
            IspPreset::TurkTelekom => BypassConfig::turk_telekom(),
            IspPreset::Vodafone => BypassConfig::vodafone_tr(),
            IspPreset::Superonline => BypassConfig::superonline(),
            IspPreset::Aggressive => BypassConfig::aggressive(),
        }
    }
}

async fn run_bypass(listen: &str, preset: &IspPreset, verbose: bool) -> Result<()> {
    let listen_addr = listen.parse()
        .with_context(|| format!("Invalid listen address: {}", listen))?;
    
    let config = ProxyConfig {
        listen_addr,
        bypass: preset.to_bypass_config(),
        verbose,
        ..Default::default()
    };
    
    let mut proxy = BypassProxy::new(config);
    proxy.run().await?;
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if !matches!(cli.command, Commands::GenConfig { .. } | Commands::Bypass { .. }) {
        setup_logging(&cli.log_level, cli.json_logs)?;
    }

    match &cli.command {
        Commands::Bypass { listen, preset, verbose } => {
            if *verbose {
                setup_logging("debug", cli.json_logs)?;
            } else {
                setup_logging("info", cli.json_logs)?;
            }
            run_bypass(listen, preset, *verbose).await?;
        }

        Commands::Run { proxy, listen } => {
            run_daemon(&cli, *proxy, listen).await?;
        }

        Commands::Start => {
            let mut client = ControlClient::new(&cli.socket);
            client.start().await?;
            println!("Engine started");
        }

        Commands::Stop => {
            let mut client = ControlClient::new(&cli.socket);
            client.stop().await?;
            println!("Engine stopped");
        }

        Commands::Status => {
            let mut client = ControlClient::new(&cli.socket);
            let status = client.status().await?;
            
            println!("Status:");
            println!("  State: {:?}", status.state);
            println!("  Running: {}", status.running);
            println!("  Active flows: {}", status.active_flows);
            println!("  Packets processed: {}", status.packets_processed);
            println!("  Bytes processed: {}", format_bytes(status.bytes_processed));
            println!("  Errors: {}", status.error_count);
            if let Some(ref err) = status.last_error {
                println!("  Last error: {}", err);
            }
            if let Some(ref path) = status.config_path {
                println!("  Config: {}", path);
            }
        }

        Commands::Health => {
            let mut client = ControlClient::new(&cli.socket);
            let health = client.health().await?;
            
            println!("Health:");
            println!("  Version: {}", health.version);
            println!("  API Version: {}", health.api_version);
            println!("  Running: {}", health.running);
            println!("  Uptime: {}s", health.uptime_secs);
            if let Some(ref backend) = health.backend {
                println!("  Backend: {}", backend);
            }
            println!("  OS: {} ({})", health.system.os, health.system.arch);
        }

        Commands::Stats => {
            let mut client = ControlClient::new(&cli.socket);
            let response = client.send(control::Command::GetStats).await?;
            
            if let control::ResponseData::Stats(stats) = response.data {
                println!("Statistics:");
                println!("  Packets in:       {}", stats.packets_in);
                println!("  Packets out:      {}", stats.packets_out);
                println!("  Bytes in:         {}", format_bytes(stats.bytes_in));
                println!("  Bytes out:        {}", format_bytes(stats.bytes_out));
                println!("  Packets dropped:  {}", stats.packets_dropped);
                println!("  Packets matched:  {}", stats.packets_matched);
                println!("  Transformed:      {}", stats.packets_transformed);
                println!("  Transform errors: {}", stats.transform_errors);
                println!("  Active flows:     {}", stats.active_flows);
                println!("  Flows created:    {}", stats.flows_created);
                println!("  Flows evicted:    {}", stats.flows_evicted);
                println!("  Fragments gen:    {}", stats.fragments_generated);
                println!("  Total jitter:     {}ms", stats.total_jitter_ms);
                println!("  Decoys sent:      {}", stats.decoys_sent);
            }
        }

        Commands::ResetStats => {
            let mut client = ControlClient::new(&cli.socket);
            client.send(control::Command::ResetStats).await?;
            println!("Statistics reset");
        }

        Commands::Validate { config } => {
            match Config::load_from_file(config) {
                Ok(_) => {
                    println!("✓ Configuration is valid: {}", config.display());
                }
                Err(e) => {
                    eprintln!("✗ Configuration error: {}", e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Reload { config } => {
            let new_config = Config::load_from_file(config)
                .with_context(|| format!("Failed to load config from {}", config.display()))?;
            
            let mut client = ControlClient::new(&cli.socket);
            client.send(control::Command::Reload(new_config)).await?;
            println!("Configuration reloaded");
        }

        Commands::GenConfig { format, output } => {
            let config = create_example_config();
            
            let content = match format.as_str() {
                "json" => serde_json::to_string_pretty(&config)?,
                "toml" | _ => toml::to_string_pretty(&config)?,
            };

            if let Some(path) = output {
                std::fs::write(path, &content)?;
                println!("Configuration written to {}", path.display());
            } else {
                println!("{}", content);
            }
        }
    }

    Ok(())
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

fn create_example_config() -> Config {
    use engine::config::*;
    use std::collections::HashMap;

    Config {
        global: GlobalConfig {
            enabled: true,
            enable_fragmentation: true,
            enable_jitter: false,
            enable_padding: true,
            enable_header_normalization: true,
            log_level: "info".to_string(),
            json_logging: false,
        },
        rules: vec![
            Rule {
                name: "https-evasion".to_string(),
                enabled: true,
                priority: 100,
                match_criteria: MatchCriteria {
                    dst_ports: Some(vec![443]),
                    protocols: Some(vec![Protocol::Tcp]),
                    ..Default::default()
                },
                transforms: vec![
                    TransformType::Fragment,
                    TransformType::Padding,
                ],
                overrides: HashMap::new(),
            },
            Rule {
                name: "dns-protection".to_string(),
                enabled: true,
                priority: 90,
                match_criteria: MatchCriteria {
                    dst_ports: Some(vec![53]),
                    protocols: Some(vec![Protocol::Udp]),
                    ..Default::default()
                },
                transforms: vec![
                    TransformType::Padding,
                ],
                overrides: HashMap::new(),
            },
        ],
        limits: Limits {
            max_flows: 10_000,
            max_queue_size: 1_000,
            max_memory_mb: 128,
            max_jitter_ms: 500,
            flow_timeout_secs: 120,
            log_rate_limit: 100,
        },
        transforms: TransformParams {
            fragment: FragmentParams {
                min_size: 1,
                max_size: 40,
                split_at_offset: None,
                randomize: true,
            },
            resegment: ResegmentParams {
                segment_size: 16,
                max_segments: 8,
            },
            padding: PaddingParams {
                min_bytes: 0,
                max_bytes: 64,
                fill_byte: None,
            },
            jitter: JitterParams {
                min_ms: 0,
                max_ms: 50,
            },
            header: HeaderParams {
                normalize_ttl: false,
                ttl_value: 64,
                normalize_window: false,
                randomize_ip_id: true,
            },
            decoy: DecoyParams {
                send_before: false,
                send_after: false,
                ttl: 1,
                probability: 0.0,
            },
        },
    }
}
