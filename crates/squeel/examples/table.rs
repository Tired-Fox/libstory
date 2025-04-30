#![allow(dead_code)]

use sqlx::{sqlite::SqlitePoolOptions, SqliteConnection, SqlitePool};
use squeel_macros::Table;

#[derive(Debug, Table)]
struct Parent {
    id: u32
}

#[derive(Debug, Table)]
#[table(rename="lower", rename_all="UPPER")]
struct Test {
    #[column(primary)]
    id: u32,
    email: String,
    age: Option<u32>
}

fn main() {
    use squeel::Table;

    let pool = SqlitePool::connect_with(SqlitePoolOptions::new())?;
    _ = Test::insert(&pool, ("testing@gmail.com".into(), None));
}
