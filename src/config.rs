use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use secrecy::SecretString;
use v_utils::macros::{LiveSettings, MyConfigPrimitives, Settings};

#[derive(Clone, Debug, LiveSettings, MyConfigPrimitives, Settings)]
pub struct Config {
	pub telegram_bot_token: SecretString,
	/// Needs Manage Channels + Manage Webhooks in the mirror category: tg-sync
	/// creates a channel per Telegram Topic and mints its own webhooks.
	pub discord_bot_token: SecretString,
	/// Bot API chat id of the supergroup, e.g. `-1001234567890`.
	pub tg_chat_id: i64,
	/// The Discord category every mirror channel is created under. The guild is
	/// read off it at boot, so there is no second id to keep in step.
	pub discord_tg_category_id: String,
	/// Inline-relay ceiling. Telegram's `getFile` refuses over 20 MB and Discord's
	/// free tier refuses over 10 MB, so the real ceiling is below both; anything
	/// larger is mirrored as a stub + source link, never dropped.
	pub tg_media_max_bytes: u64 = 8 * 1024 * 1024,
	/// Long-poll window handed to `getUpdates`. The HTTP timeout is derived from it.
	pub tg_poll_timeout_secs: u64 = 25,
	pub bind: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 55680)),
}
