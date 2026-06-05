use reqwest::blocking::Client;
use serde_json::Value;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder()
        .user_agent("reqwest-gh-stars-co2-example")
        .build()?;

    let resp = client
        .get("https://api.github.com/repos/rust-lang/rust")
        .send()?;
    let repo: Value = resp.json()?;

    let name = repo
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");

    let stars = repo
        .get("stargazers_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    println!("{name} has {stars} stars");

    Ok(())
}
