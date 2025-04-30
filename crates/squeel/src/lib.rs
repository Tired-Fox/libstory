use std::{borrow::Cow, cmp::Ordering, path::{Path, PathBuf}, sync::Arc, time::Duration};
use sqlx::{migrate::Migrator, sqlite::SqliteConnectOptions, Pool, Sqlite, SqlitePool};

pub use squeel_macros::Table;

pub trait Table: Sized {
    type New;
    type Fetch;
    type Keys;

    fn table_name() -> &'static str;
    fn column_name(name: &str) -> &str { name }
    fn scaffold() -> String;
    fn create(pool: &Pool<Sqlite>) -> impl Future<Output = sqlx::Result<()>>;
    fn insert(pool: &Pool<Sqlite>, args: Self::New) -> impl Future<Output = sqlx::Result<Self::Keys>>;
    fn update(&self, pool: &Pool<Sqlite>) -> impl Future<Output = sqlx::Result<Self::Keys>>;
    fn delete(&self, pool: &Pool<Sqlite>) -> impl Future<Output = sqlx::Result<Self::Keys>>;
    fn one(pool: &Pool<Sqlite>, fetch: Self::Fetch) -> impl Future<Output = sqlx::Result<Self>>;
    fn many(pool: &Pool<Sqlite>, fetch: Self::Fetch) -> impl Future<Output = sqlx::Result<Vec<Self>>>;
}

pub struct Connection {
    pub pool: Pool<Sqlite>
}

impl Connection {
    pub async fn new(options: SqliteConnectOptions) -> anyhow::Result<Self> {
        let pool = SqlitePool::connect_with(options).await?;
        Ok(Self { pool })
    }

    pub fn builder() -> ConnectionBuilder {
        ConnectionBuilder::default()
    }

    pub async fn create_table<T: Table>(&self) -> anyhow::Result<()> {
        Ok(T::create(&self.pool).await?)
    }

    pub async fn insert<T: Table>(&self, args: T::New) -> anyhow::Result<T::Keys> {
        Ok(T::insert(&self.pool, args).await?)
    }

    pub async fn update<T: Table>(&self, item: &T) -> anyhow::Result<T::Keys> {
        Ok(item.update(&self.pool).await?)
    }

    pub async fn delete<T: Table>(&self, item: &T) -> anyhow::Result<T::Keys> {
        Ok(item.delete(&self.pool).await?)
    }

    pub async fn one<T: Table>(&self, filter: T::Fetch) -> anyhow::Result<T> {
        Ok(T::one(&self.pool, filter).await?)
    }

    pub async fn many<T: Table>(&self, filter: T::Fetch) -> anyhow::Result<Vec<T>> {
        Ok(T::many(&self.pool, filter).await?)
    }
}

#[derive(Default, Debug)]
pub struct ConnectionBuilder {
    options: SqliteConnectOptions,
    migrator: Option<Migrator>,
}

impl ConnectionBuilder {
    pub fn filename(mut self, filename: impl AsRef<Path>) -> Self {
        self.options = self.options.filename(filename);
        self
    }

    pub fn create_if_missing(mut self, create: bool) -> Self {
        self.options = self.options.create_if_missing(create);
        self
    }

    pub fn in_memory(mut self, in_memory: bool) -> Self {
        self.options = self.options.in_memory(in_memory);
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.options = self.options.read_only(read_only);
        self
    }

    pub fn shared_cache(mut self, shared_cache: bool) -> Self {
        self.options = self.options.shared_cache(shared_cache);
        self
    }

    pub fn statement_cache_capacity(mut self, statement_cache_capacity: usize) -> Self {
        self.options = self.options.statement_cache_capacity(statement_cache_capacity);
        self
    }

    pub fn busy_timeout(mut self, busy_timeout: Duration) -> Self {
        self.options = self.options.busy_timeout(busy_timeout);
        self
    }

    pub fn immutable(mut self, immutable: bool) -> Self {
        self.options = self.options.immutable(immutable);
        self
    }

    pub fn vfs(mut self, vfs: impl Into<Cow<'static, str>>) -> Self {
        self.options = self.options.vfs(vfs);
        self
    }

    pub fn pragma<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<Cow<'static, str>>,
        V: Into<Cow<'static, str>>,
    {
        self.options = self.options.pragma(key, value);
        self
    }

    pub fn extension(mut self, extension: impl Into<Cow<'static, str>>) -> Self {
        self.options = self.options.extension(extension);
        self
    }

    pub fn extension_with_entrypoint<N, E>(mut self, name: N, entry: E) -> Self
    where
        N: Into<Cow<'static, str>>,
        E: Into<Cow<'static, str>>,
    {
        self.options = self.options.extension_with_entrypoint(name, entry);
        self
    }

    pub fn command_buffer_size(mut self, command_buffer_size: usize) -> Self {
        self.options = self.options.command_buffer_size(command_buffer_size);
        self
    }

    pub fn row_buffer_size(mut self, row_buffer_size: usize) -> Self {
        self.options = self.options.row_buffer_size(row_buffer_size);
        self
    }

    pub fn collation<N, F>(mut self, name: N, collate: F) -> Self
    where
        N: Into<Arc<str>>,
        F: Fn(&str, &str) -> Ordering + Send + Sync + 'static,
    {
        self.options = self.options.collation(name, collate);
        self
    }

    pub fn serializated(mut self, serialized: bool) -> Self {
        self.options = self.options.serialized(serialized);
        self
    }

    pub fn thread_name(mut self, generator: impl Fn(u64) -> String + Send + Sync + 'static) -> Self {
        self.options = self.options.thread_name(generator);
        self
    }

    pub fn optimize_on_close(mut self, enabled: bool, analysis_limit: impl Into<Option<u32>>) -> Self {
        self.options = self.options.optimize_on_close(enabled, analysis_limit);
        self
    }

    pub async fn migrations(mut self, migrator: impl IntoMigrator) -> anyhow::Result<Self> {
        self.migrator = Some(migrator.into_migrator().await?);
        Ok(self)
    }

    pub async fn build(self) -> anyhow::Result<Connection> {
        let pool = SqlitePool::connect_with(self.options).await?;
        if let Some(migrator) = self.migrator {
            migrator.run(&pool).await?;
        }
        Ok(Connection { pool })
    }
}

pub trait IntoMigrator {
    fn into_migrator(self) -> impl Future<Output = anyhow::Result<Migrator>>;
}

impl IntoMigrator for &str {
    async fn into_migrator(self) -> anyhow::Result<Migrator> {
        Ok(Migrator::new(PathBuf::from(self)).await?)
    }
}

impl IntoMigrator for String {
    async fn into_migrator(self) -> anyhow::Result<Migrator> {
        Ok(Migrator::new(PathBuf::from(self)).await?)
    }
}

impl IntoMigrator for PathBuf {
    async fn into_migrator(self) -> anyhow::Result<Migrator> {
        Ok(Migrator::new(self).await?)
    }
}

impl IntoMigrator for Migrator {
    async fn into_migrator(self) -> anyhow::Result<Migrator> {
        Ok(self)
    }
}
