use engine::DohResolver;

#[tokio::main]
async fn main() {
    let resolver = DohResolver::new();

    for host in ["discord.com", "x.com", "wikipedia.org"] {
        match resolver.resolve(host).await {
            Ok(ips) => println!("{} -> {:?}", host, ips),
            Err(e) => println!("{} FAILED: {}", host, e),
        }
    }

    println!("cached entries: {}", resolver.cache_len());
    println!(
        "{:?}",
        resolver.resolve_socket_addrs("discord.com:443").await
    );
}
