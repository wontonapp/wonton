use crate::{
    service::App,
    store::{Store, hash, now, random_token},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};

#[derive(Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "lowercase")]
pub enum Command {
    Invite { valid_seconds: u64 },
    Suspend { home: String, reason: String },
    Resume { home: String },
}

pub async fn execute(app: &App, command: Command) -> Result<Value> {
    let mut store = app.store.lock().await;
    match command {
        Command::Invite { valid_seconds } => {
            if valid_seconds == 0 || valid_seconds > 2592000 {
                bail!("invite lifetime must be 1..2592000 seconds");
            }
            let token = random_token()?;
            store.data.invites.retain(|_, expiry| *expiry >= now());
            store
                .data
                .invites
                .insert(hash(token.as_bytes()), now() + valid_seconds);
            store.save()?;
            Ok(json!({"invite":token}))
        }
        Command::Suspend { home, reason } => {
            if reason.is_empty() || reason.len() > 1024 {
                bail!("suspension reason required, maximum 1024 bytes");
            }
            let record = store.data.homes.get_mut(&home).context("unknown home")?;
            record.suspended = true;
            record.reason = Some(reason);
            store.save()?;
            app.dns.reconcile(&home, &store.data.homes[&home]).await?;
            Ok(json!({}))
        }
        Command::Resume { home } => {
            let record = store.data.homes.get_mut(&home).context("unknown home")?;
            if record.deleted {
                bail!("deleted home cannot resume");
            }
            // Hooks remain closed until DNS restoration succeeds.
            let mut restored = record.clone();
            restored.suspended = false;
            restored.reason = None;
            app.dns.reconcile(&home, &restored).await?;
            store.data.homes.insert(home, restored);
            store.save()?;
            Ok(json!({}))
        }
    }
}

async fn connection(app: Arc<App>, stream: UnixStream) -> Result<()> {
    let (read, mut write) = stream.into_split();
    use tokio::io::AsyncReadExt;
    let mut read = BufReader::new(read.take(16385));
    let mut line = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        read.read_until(b'\n', &mut line),
    )
    .await??;
    if line.len() > 16384 {
        bail!("admin request too large");
    }
    let result = match serde_json::from_slice::<Command>(&line) {
        Ok(command) => match execute(&app, command).await {
            Ok(value) => json!({"ok":value}),
            Err(error) => json!({"error":error.to_string()}),
        },
        Err(_) => json!({"error":"invalid admin command"}),
    };
    write.write_all(&serde_json::to_vec(&result)?).await?;
    write.write_all(b"\n").await?;
    Ok(())
}

pub async fn serve(app: Arc<App>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let socket = Store::socket_path(&app.config.state);
    match std::fs::remove_file(&socket) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    loop {
        let (stream, _) = listener.accept().await?;
        let app = app.clone();
        tokio::spawn(async move {
            if let Err(error) = connection(app, stream).await {
                tracing::warn!(%error, "admin request failed");
            }
        });
    }
}

pub async fn client(state: &Path, args: &[String]) -> Result<()> {
    let command = match args {
        [op, valid] if op == "invite" => Command::Invite {
            valid_seconds: valid.parse()?,
        },
        [op, home, reason] if op == "suspend" => Command::Suspend {
            home: home.clone(),
            reason: reason.clone(),
        },
        [op, home] if op == "resume" => Command::Resume { home: home.clone() },
        _ => bail!(
            "usage: wonton-home [serve | invite <seconds> | suspend <home> <reason> | resume <home>]"
        ),
    };
    let mut stream = UnixStream::connect(Store::socket_path(state))
        .await
        .context("connect to running home service")?;
    stream.write_all(&serde_json::to_vec(&command)?).await?;
    stream.write_all(b"\n").await?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).await?;
    let value: Value = serde_json::from_str(&response)?;
    if let Some(error) = value.get("error") {
        bail!("admin operation failed: {error}");
    }
    if let Some(invite) = value["ok"]["invite"].as_str() {
        println!("{invite}");
    } else {
        println!("{}", value["ok"]);
    }
    Ok(())
}
