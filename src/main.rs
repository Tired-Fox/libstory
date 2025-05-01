use squeel::Connection;

#[derive(Debug, squeel::Table)]
struct Manga {
    #[column(primary)]
    pub id: u32,
    #[column(unique)]
    pub name: String,
    pub volumes: u32,
}

#[derive(Debug, squeel::Table)]
struct Volume {
    #[column(primary)]
    pub id: u32,
    #[column(foreign=Manga(id))]
    pub manga_id: u32,
    #[column(unique)]
    pub name: String,
    pub read_state: ReadState,
    pub part: Option<u32>,
}

#[derive(sqlx::Type, Default, Debug)]
enum ReadState {
    #[default]
    Unread,
    Reading,
    Complete,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let conn = Connection::builder()
        .filename("manga.sqlite")
        .create_if_missing(true)
        .migrations("src/migrations")
        .await?
        .build()
        .await?;

    _ = sqlx::query("")
        .bind(ReadState::Unread);

    conn.create_table::<Manga>().await?;
    conn.create_table::<Volume>().await?;

    let (id,)= match conn.one::<Manga>((None, Some("Solo Leveling".into()), None)).await {
        Ok(manga) => (manga.id,),
        Err(_) => conn.insert::<Manga>(("Solo Leveling".into(), 12)).await?,
    };

    let manga = conn.many::<Manga>((None, Some("Solo Leveling".into()), None)).await?;
    println!("[{id}] {manga:#?}");


    let (volume_id, manga_id)= match conn.one::<Volume>((None, Some(id), Some("1".into()), None, None)).await {
        Ok(volume) => (volume.id, volume.manga_id),
        Err(_) => conn.insert::<Volume>((id, "1".into(), Default::default(), None)).await?,
    };

    let volumes = conn.many::<Volume>((None, Some(id), None, None, None)).await?;
    println!("[{manga_id}:{volume_id}] {volumes:#?}");

    Ok(())
}
