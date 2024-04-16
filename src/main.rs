extern crate bitcoin;
extern crate num_cpus;
extern crate secp256k1;

use std::fs::OpenOptions;
use std::io::BufRead;
use std::sync::{Arc, RwLock};
use std::{
    collections::HashSet,
    fs::File,
    io::Write,
    time::Instant,
};

use bitcoin::Address;
use bitcoin::{network::constants::Network, PrivateKey, PublicKey};
use secp256k1::{rand, Secp256k1, SecretKey};

use tokio::task;

// const DB_VER: &str = "MAR_15_2021";
// const DB_VER: &str = "4_5_2024";
const DB_DIR: &str = "Bitcoin_addresses_LATEST.txt"; // 将 4_5_2024 目录包含到二进制文件中

const SUB_STRING_COUNT: i32 = 8;

#[tokio::main]
async fn main() {
    check_and_create_file();
    // creating empty database
    let mut database = HashSet::new();
    let timer = Instant::now();
    let data: Vec<String> = load_address_in_txt(DB_DIR);
    // adding addresses to database
    for ad in data.iter() {
        database.insert(ad.to_string());
    }
    println!("Database size {:?} addresses.", database.len());

    println!(
        "Load of  files completed in {:.2?}, database size: {:?}",
        timer.elapsed(),
        database.len()
    );

    // single thread version of processing
    // process(&database);

    // Multithread version of processing using tokio
    // atomic reference counting of database
    let database_ = Arc::new(RwLock::new(database));
    //get number of logical cores
    let num_cores = num_cpus::get();
    println!("Running on {} logical cores", num_cores);
    //run process on all available cores
    for _ in 0..num_cores {
        let clone_database_ = Arc::clone(&database_);
        task::spawn_blocking(move || {
            let current_core = std::thread::current().id();
            println!("Core {:?} started", current_core);
            let db = clone_database_.read().unwrap();
            process(&db);
        });
    }
}

// load single txt file from database directory
fn load_address_in_txt(path: &str) -> Vec<String> {
    let file = File::open(path).expect("couldn't open address file");
    let reader = std::io::BufReader::new(file);
    let mut addresses: Vec<String> = Vec::new();
    for line in reader.lines() {
        if let Ok(address) = line {
            if address.starts_with("1") {
                addresses.push(
                    address[(address.chars().count() - SUB_STRING_COUNT as usize)..].to_string(),
                );
            }
        }
    }
    addresses
}

fn check_and_create_file() {
    let file_path = found_file_path();
    if !std::path::Path::new(&file_path).exists() {
        let _file = std::fs::File::create(&file_path).unwrap();
        // You can write some initial content to the file here if needed
        println!("Created new plutus.txt file.");
    } else {
        println!("plutus.txt file already exists.");
    }
}

// write data to file
fn write_to_file(data: &str, file_name: &str) {
    let mut file = OpenOptions::new()
        .append(true)
        .open(file_name)
        .expect("Unable to open file");
    file.write_all(data.as_bytes()).unwrap();
}

// function that checks address in database and if finds it, writes data to file
fn check_address(
    private_key: &PrivateKey,
    secret_key: SecretKey,
    address: &Address,
    database: &HashSet<String>,
    public_key: PublicKey,
) {
    // let _control_address = "11111111111111111111HV1eYjP".to_string();
    // let address_string = _control_address;
    let address_string = address.to_string();
    let address_suffix =
        &address_string[(address_string.chars().count() - SUB_STRING_COUNT as usize)..];
    if database.contains(address_suffix) {
        let data = format!(
            "{}{}{}{}{}{}{}{}{}",
            secret_key.display_secret(),
            "\n",
            private_key.to_wif(),
            "\n",
            public_key.to_string(),
            "\n",
            address_string.as_str(),
            "\n",
            "\n",
        );
        write_to_file(data.as_str(), found_file_path().as_str());
    }
}

// get found.txt file path
fn found_file_path() -> String {
    let mut path = std::env::current_dir().unwrap();
    path.push("plutus.txt");
    path.to_str().unwrap().to_string()
}

// infinite loop processing function
fn process(database: &HashSet<String>) {
    let mut count: f64 = 0.0;
    let start = Instant::now();
    loop {
        // Generating secret key
        let secp = Secp256k1::new();
        let secret_key = SecretKey::new(&mut rand::thread_rng());
        let private_key = PrivateKey::new(secret_key, Network::Bitcoin);
        let public_key = PublicKey::from_private_key(&secp, &private_key);
        // Generate pay-to-pubkey-hash (P2PKH) wallet address
        let address = Address::p2pkh(&public_key, Network::Bitcoin);

        // check address against database
        check_address(&private_key, secret_key, &address, database, public_key);

        // FOR BENCHMARKING ONLY! (has to be commented out for performance gain)
        count += 1.0;
        if count % 100000.0 == 0.0 {
            let current_core = std::thread::current().id();
            let elapsed = start.elapsed().as_secs_f64();
            println!(
                "Core {:?} checked {} addresses in {:.2?}, iter/sec: {}",
                current_core,
                count,
                elapsed,
                count / elapsed
            );
        }
    }
}
