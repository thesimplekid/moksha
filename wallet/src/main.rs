use std::env;

use cashurs_core::{
    dhke,
    model::{BlindedMessage, BlindedSignature, Proof, Token, Tokens},
};
use clap::{Parser, Subcommand};
use dotenvy::dotenv;
use rand::{distributions::Alphanumeric, Rng};
use secp256k1::SecretKey;

mod client;

#[derive(Parser)]
#[command(version)]
struct Opts {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand, Clone)]
enum Command {
    Balance,
    Invoice { amount: u64 },
    Send { amount: u64 },
    Pay { invoice: String },
    Info,
}

fn generate_random_string() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(24)
        .map(char::from)
        .collect()
}

fn read_env() -> String {
    dotenv().expect(".env file not found");
    env::var("MINT_URL").expect("MINT_URL not found")
}

fn wait_for_payment(invoice: String) {
    println!("Pay invoice to mint sats. Press return after invoice is paid:\n\n{invoice}");
    loop {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .expect("Error: Could not read a line");
        if line == "\n" {
            break;
        }
    }
}

/// split a decimal amount into a vector of powers of 2
fn split_amount(amount: u64) -> Vec<u64> {
    format!("{:b}", amount)
        .chars()
        .rev()
        .enumerate()
        .filter_map(|(i, c)| {
            if c == '1' {
                return Some(2_u64.pow(i as u32));
            }
            None
        })
        .collect::<Vec<u64>>()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mint_url = read_env();

    let client = client::Client::new(mint_url.clone());
    let keys = client.get_mint_keys().await.unwrap();
    let keysets = client.get_mint_keysets().await;

    let cli = Opts::parse();

    match cli.command {
        Command::Invoice { amount } => {
            let payment_request = client.get_mint_payment_request(amount).await;
            let payment_hash = payment_request.clone().unwrap().hash;

            let invoice = payment_request.unwrap().pr;
            wait_for_payment(invoice);

            let split_amount = split_amount(amount);

            let secrets = (0..split_amount.len())
                .map(|_| generate_random_string())
                .collect::<Vec<String>>();

            let blinded_messages = split_amount
                .into_iter()
                .zip(secrets.clone())
                .map(|(amount, secret)| {
                    let (b_, alice_secret_key) = dhke::step1_alice(secret, None).unwrap();
                    (BlindedMessage { amount, b_ }, alice_secret_key)
                })
                .collect::<Vec<(BlindedMessage, SecretKey)>>();

            let post_mint_resp = client
                .post_mint_payment_request(
                    payment_hash,
                    blinded_messages
                        .clone()
                        .into_iter()
                        .map(|(msg, _)| msg)
                        .collect::<Vec<BlindedMessage>>(),
                )
                .await
                .unwrap();

            // step 3: unblind signatures
            let keysets = keysets.unwrap().keysets;
            let current_keyset = keysets[keysets.len() - 1].clone();

            let private_keys = blinded_messages
                .clone()
                .into_iter()
                .map(|(_, secret)| secret)
                .collect::<Vec<SecretKey>>();

            let proofs = post_mint_resp
                .promises
                .iter()
                .zip(private_keys)
                .zip(secrets)
                .map(|((p, priv_key), secret)| {
                    let key = keys
                        .get(&p.amount)
                        .expect("msg amount not found in mint keys");
                    let pub_alice = dhke::step3_alice(p.c_, priv_key, *key);
                    Proof::new(p.amount, secret, pub_alice, current_keyset.clone())
                })
                .collect::<Vec<Proof>>();

            let serialized_tokens = Tokens::new(Token {
                mint: Some(mint_url.to_string()),
                proofs,
            })
            .serialize()
            .unwrap();

            println!("Minted tokens:\n\n{serialized_tokens}");
        }
        Command::Pay { invoice } => {
            println!("Pay {invoice}");
        }
        Command::Info => {
            println!("Info");
        }
        Command::Balance => {
            println!("Balance");
        }
        Command::Send { amount } => {
            println!("Send {amount}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_split() -> anyhow::Result<()> {
        let amount = 13;
        let bits = super::split_amount(amount);
        assert_eq!(bits, vec![1, 4, 8]);
        Ok(())
    }
}