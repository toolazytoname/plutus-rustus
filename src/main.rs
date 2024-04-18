extern crate bitcoin;
extern crate num_cpus;
extern crate secp256k1;

use std::fs::OpenOptions;
use std::sync::{Arc, RwLock};
use std::{
    fs::File,
    io::Write,
    time::Instant,
};
use bitcoin::Address;
use bitcoin::{network::constants::Network, PrivateKey, PublicKey};
use secp256k1::{rand, Secp256k1, SecretKey};
use tokio::task;
use fastbloom_rs::{BloomFilter, FilterBuilder, Membership};
use csv::ReaderBuilder;
use rusqlite::{Connection, Result};
use rusqlite::params;

const TSV_DIR: &str = "blockchair_bitcoin_addresses_and_balance_LATEST.tsv";// 2024_4_18
const DB_DIR: &str = "bitcoin.db";

#[tokio::main]
async fn main() {
    check_and_create_file();
    let timer = Instant::now();
    //check sqlite
    let load_tsv_result = load_address_and_balance_in_tsv();
    println!("Load tsv completed in {:.2?};result is {:?}",timer.elapsed(),load_tsv_result);
    let filter:BloomFilter = load_bloom_in_sqlite(DB_DIR);
    println!("Create Bloom completed in {:.2?}",timer.elapsed());

    // single thread version of processing
    // process(&database);

    // Multithread version of processing using tokio
    // atomic reference counting of database
    let filter_ = Arc::new(RwLock::new(filter));
    //get number of logical cores
    let num_cores = num_cpus::get();
    // let num_cores = 2;
    println!("Running on {} logical cores", num_cores);
    //run process on all available cores
    for _ in 0..num_cores {
        let clone_filter_ = Arc::clone(&filter_);
        task::spawn_blocking(move || {
            let current_core = std::thread::current().id();
            println!("Core {:?} started", current_core);
            let fl = clone_filter_.read().unwrap();
            process(&fl);
        });
    }
}

fn load_address_and_balance_in_tsv() -> Result<(), Box<dyn std::error::Error>> {
    if !std::path::Path::new(TSV_DIR).exists(){
        println!("tsv file not found in {}",TSV_DIR);
        // return Err("tsv file not found".into());
    }
    println!("tsv file found in {}",TSV_DIR);
    //check if db file exists
    
    if std::path::Path::new(DB_DIR).exists(){
        println!("db file already exists in {}",DB_DIR);
        let conn = Connection::open(DB_DIR)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM btc_addresses", [], |row| row.get(0))?;
        println!("db already exists in {},Total number of rows in btc_addresses table: {}", DB_DIR,count);
        return Ok(());
    }
    println!("Create db in {} ",DB_DIR);
    println!("Create table ");
    let mut conn = Connection::open(DB_DIR)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS btc_addresses (address TEXT PRIMARY KEY, balance INTEGER)",
        [],
    )?;
    println!("Insert data into table ");
    let tx = conn.transaction().unwrap();
    let file = File::open(TSV_DIR).expect("couldn't open tsv file");
    let mut rdr = ReaderBuilder::new().delimiter(b'\t').from_reader(file);
    for result in rdr.records() {
        let record = result?;
        let address = record.get(0).unwrap();
        let balance = record.get(1).unwrap();
        // println!("{:?}", record);
        tx.execute(
            "INSERT INTO btc_addresses (address, balance) VALUES (?1,?2)",
            &[&address, &balance],
        )?;
    }
    let tx_result = tx.commit();
    println!("Insert data into table end.{:?}",tx_result);
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM btc_addresses", [], |row| row.get(0))?;
    println!("Total number of rows in btc_addresses table: {}",count);
    println!("crreate index.");
    // 创建 address 字段的索引
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_address ON btc_addresses (address)",
        [],
    )?;
    println!("load_address_and_balance_in_tsv end.");
    Ok(())
}

fn load_bloom_in_sqlite(sqlite_db_path: &str) -> BloomFilter {
    let conn = Connection::open(sqlite_db_path).unwrap();
    let mut addresses = FilterBuilder::new(53_273_531, 0.000_001).build_bloom_filter();
    let mut stmt = conn.prepare("SELECT address FROM btc_addresses").unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    for row in rows {
        let address = row.unwrap();
        addresses.add(address.as_bytes());
    }
    println!("load_bloom_in_sqlite end.");
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
    filter: &BloomFilter,
    public_key: PublicKey,

) {
    //check Bloom first
    let bloom_may_contain = filter.contains(address.to_string().as_bytes());
    if  bloom_may_contain{

        let data = format!(
            "secret_key:{}\n private_key:{} \n public_key:{} \naddress:{}\n",
            secret_key.display_secret(),
            private_key.to_wif(),
            public_key.to_string(),
            address.to_string().as_str(),
        );
        println!("Bloom Found data: {}", data);

        let conn = Connection::open(DB_DIR).unwrap();
        let mut stmt = conn.prepare("SELECT balance FROM btc_addresses WHERE address = ?").unwrap();
        let mut rows = stmt.query(params![address.to_string()]).unwrap();
    
        if let Some(row) = rows.next().unwrap() {
            let balance: i64 = row.get(0).unwrap();
            let data = format!(
                "secret_key:{}\n private_key:{} \n public_key:{} \naddress:{}\n balance:{} \n\n",
                secret_key.display_secret(),
                private_key.to_wif(),
                public_key.to_string(),
                address.to_string().as_str(),
                balance
            );
            println!("sqlite Found data: {}", data);
            write_to_file(data.as_str(), found_file_path().as_str());
        } else {
            println!("Address {} does not exist in the database.\n\n", address);
        }
        
    }
}

// get found.txt file path
fn found_file_path() -> String {
    let mut path = std::env::current_dir().unwrap();
    path.push("plutus.txt");
    path.to_str().unwrap().to_string()
}

// infinite loop processing function
//hashmap
// Core ThreadId(11) checked 100000 addresses in 1.90, iter/sec: 52702.73110164057
// Core ThreadId(10) checked 100000 addresses in 1.90, iter/sec: 52628.38545693889
//Bloom
 // Core ThreadId(5) checked 100000 addresses in 1.98, iter/sec: 50403.73601725324
 // Core ThreadId(2) checked 100000 addresses in 1.99, iter/sec: 50322.380897775176

//  Core ThreadId(4) checked 100000 addresses in 3.05, iter/sec: 32773.38471749095
fn process(filter: &BloomFilter) {
    // let mut count: f64 = 0.0;
    // let start = Instant::now();
    loop {
        // Generating secret key
        let secp = Secp256k1::new();
        let secret_key = SecretKey::new(&mut rand::thread_rng());
        let private_key = PrivateKey::new(secret_key, Network::Bitcoin);
        let public_key = PublicKey::from_private_key(&secp, &private_key);
        // Generate pay-to-pubkey-hash (P2PKH) wallet address
        let address = Address::p2pkh(&public_key, Network::Bitcoin);
    // let _control_address = "11111111111111111111HV1eYjP".to_string();
        // check address against database
        check_address(&private_key, secret_key, &address, filter, public_key);

        // FOR BENCHMARKING ONLY! (has to be commented out for performance gain)
        // count += 1.0;
        // if count % 100000.0 == 0.0 {
        //     let current_core = std::thread::current().id();
        //     let elapsed = start.elapsed().as_secs_f64();
        //     println!(
        //         "Core {:?} checked {} addresses in {:.2?}, iter/sec: {}",
        //         current_core,
        //         count,
        //         elapsed,
        //         count / elapsed
        //     );
        // }
    }
}
