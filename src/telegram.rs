use std::time::Duration;

use color_eyre::eyre::{Result, eyre};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Update {
	pub update_id: i64,
	pub message: Option<Message>,
	pub edited_message: Option<Message>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
	pub message_id: i64,
	pub message_thread_id: Option<i64>,
	pub chat: Chat,
	pub from: Option<User>,
	pub text: Option<String>,
	pub caption: Option<String>,
	pub reply_to_message: Option<Box<Message>>,
	pub forum_topic_created: Option<ForumTopicCreated>,
	pub forum_topic_edited: Option<ForumTopicEdited>,
	/// Same photo in ascending sizes; the relay picks the largest that fits.
	pub photo: Option<Vec<PhotoSize>>,
	pub video: Option<FileMeta>,
	pub document: Option<FileMeta>,
	pub audio: Option<FileMeta>,
	pub voice: Option<FileMeta>,
	pub video_note: Option<FileMeta>,
	pub animation: Option<FileMeta>,
	pub sticker: Option<FileMeta>,
}

#[derive(Debug, Deserialize)]
pub struct Chat {
	pub id: i64,
	/// Absent on everything that is not a supergroup with Topics enabled.
	pub is_forum: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct User {
	pub first_name: String,
	pub last_name: Option<String>,
}
impl User {
	/// Display name for the Discord webhook identity override.
	pub fn display(&self) -> String {
		match &self.last_name {
			Some(last) => format!("{} {last}", self.first_name),
			None => self.first_name.clone(),
		}
	}
}

#[derive(Debug, Deserialize)]
pub struct ForumTopicCreated {
	pub name: String,
}

/// A rename carries only what changed, so `name` is absent when the edit touched
/// just the icon.
#[derive(Debug, Deserialize)]
pub struct ForumTopicEdited {
	pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PhotoSize {
	pub file_id: String,
	pub file_size: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct FileMeta {
	pub file_id: String,
	pub file_name: Option<String>,
	pub mime_type: Option<String>,
	pub file_size: Option<u64>,
	/// Stickers only.
	pub emoji: Option<String>,
}

pub struct Telegram {
	http: reqwest::Client,
	api: String,
	token: String,
	poll_timeout: u64,
}
impl Telegram {
	pub fn try_new(token: String, poll_timeout: u64) -> Result<Self> {
		// The long poll holds the connection for `poll_timeout`; the client budget
		// must clear it or every idle poll would look like a network failure.
		let http = reqwest::Client::builder().timeout(Duration::from_secs(poll_timeout + 35)).build()?;
		Ok(Self { http, api: "https://api.telegram.org".to_string(), token, poll_timeout })
	}

	/// Long-poll. `offset` doubles as the acknowledgement: Telegram drops every
	/// update below it server-side, which is why the caller must only advance it
	/// past a message that already reached Discord.
	pub async fn get_updates(&self, offset: i64) -> Result<Vec<Update>> {
		self.call(
			"getUpdates",
			&serde_json::json!({
				"offset": offset,
				"timeout": self.poll_timeout,
				"allowed_updates": ["message", "edited_message"],
			}),
		)
		.await
	}

	/// `getFile` + fetch. Refuses over 20 MB on Telegram's side, which is why the
	/// caller checks `file_size` against its own (lower) cap first.
	pub async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
		let file: File = self.call("getFile", &serde_json::json!({ "file_id": file_id })).await?;
		let path = file.file_path.ok_or_else(|| eyre!("getFile returned no file_path for {file_id}"))?;
		let bytes = self
			.http
			.get(format!("{}/file/bot{}/{path}", self.api, self.token))
			.send()
			.await
			.and_then(reqwest::Response::error_for_status)
			.map_err(reqwest::Error::without_url)?
			.bytes()
			.await
			.map_err(reqwest::Error::without_url)?;
		Ok(bytes.to_vec())
	}

	/// Reads the body before checking the status: a Bot API failure is an HTTP 4xx
	/// whose `description` is the only thing that says *why* (a revoked token, a
	/// second poller holding the same offset, a file over 20 MB).
	async fn call<T: serde::de::DeserializeOwned>(&self, method: &str, body: &serde_json::Value) -> Result<T> {
		let response = self.http.post(format!("{}/bot{}/{method}", self.api, self.token)).json(body).send().await.map_err(reqwest::Error::without_url)?;
		let status = response.status();
		// Keeps a 5xx/429 recognisable to `crate::transient` under the description.
		let status_error = response.error_for_status_ref().err().map(reqwest::Error::without_url);
		let text = response.text().await.map_err(reqwest::Error::without_url)?;
		let fail = |message: String| match status_error {
			Some(e) => color_eyre::Report::new(e).wrap_err(message),
			None => eyre!(message),
		};
		let parsed: ApiResponse<T> = match serde_json::from_str(&text) {
			Ok(parsed) => parsed,
			Err(e) => return Err(fail(format!("telegram {method} returned an unparseable body (status {status}): {e}: {text}"))),
		};
		if !parsed.ok {
			return Err(fail(format!("telegram {method} failed (status {status}): {}", parsed.description.unwrap_or_default())));
		}
		parsed.result.ok_or_else(|| eyre!("telegram {method} returned ok without a result"))
	}
}

#[derive(Deserialize)]
struct ApiResponse<T> {
	ok: bool,
	result: Option<T>,
	description: Option<String>,
}

#[derive(Deserialize)]
struct File {
	file_path: Option<String>,
}

#[cfg(test)]
mod tests {
	use tokio::io::{AsyncReadExt, AsyncWriteExt};

	use super::*;

	/// Answers one request per scripted reply; `None` drops the connection mid-request.
	async fn telegram_answering(replies: Vec<Option<&'static str>>) -> (Telegram, impl Future<Output = ()>) {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let api = format!("http://{}", listener.local_addr().unwrap());
		let server = async move {
			for reply in replies {
				let (mut socket, _) = listener.accept().await.unwrap();
				socket.read(&mut [0u8; 4096]).await.unwrap();
				if let Some(reply) = reply {
					socket.write_all(reply.as_bytes()).await.unwrap();
				}
			}
		};
		(Telegram { api, ..Telegram::try_new("TOKEN".to_string(), 0).unwrap() }, server)
	}

	#[tokio::test]
	async fn only_a_network_blip_or_a_telegram_outage_is_transient() {
		let (telegram, server) = telegram_answering(vec![
			None,
			Some("HTTP/1.1 502 Bad Gateway\r\ncontent-length: 5\r\n\r\nnginx"),
			Some("HTTP/1.1 401 Unauthorized\r\ncontent-length: 41\r\n\r\n{\"ok\":false,\"description\":\"Unauthorized\"}"),
		])
		.await;
		let client = async {
			let reset = telegram.get_updates(0).await.unwrap_err();
			assert!(crate::transient(&reset), "{reset:?}");
			assert!(!format!("{reset:?}").contains("TOKEN"), "the bot token leaked into {reset:?}");
			let outage = telegram.get_updates(0).await.unwrap_err();
			assert!(crate::transient(&outage), "{outage:?}");
			let revoked = telegram.get_updates(0).await.unwrap_err();
			assert!(!crate::transient(&revoked), "{revoked:?}");
		};
		tokio::join!(server, client);
	}
}
