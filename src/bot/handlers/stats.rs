use teloxide::prelude::*;
use teloxide::types::ReplyParameters;
use sysinfo::{System, Pid};

use crate::bot::error::BotError;

/// Formatting helper to convert bytes to human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.2} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Create a text-based progress bar for RAM usage.
fn create_progress_bar(used: u64, total: u64, width: usize) -> String {
    if total == 0 { return " ".repeat(width); }
    let ratio = used as f64 / total as f64;
    let filled_len = (ratio * width as f64).round() as usize;
    let empty_len = width.saturating_sub(filled_len);
    format!("[{}{}]", "=".repeat(filled_len), "-".repeat(empty_len))
}

/// Handle the `/stats` command.
///
/// This command provides real-time system and process information using sysinfo 0.38.
pub async fn handle_stats(
    bot: teloxide::adaptors::Throttle<Bot>,
    msg: Message,
    owner_id: Option<i64>,
) -> Result<(), BotError> {
    // Check if the user is the owner
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64);
    
    if let Some(owner) = owner_id
        && user_id != Some(owner) {
            // Silently ignore or send a permission error
            // bot.send_message(msg.chat.id, "❌ You are not authorized to use this command.").await?;
            return Ok(());
        }

    // In sysinfo 0.30+, System::new_all() initializes and refreshes everything.
    let mut sys = System::new_all();
    
    // Initial refresh is done by new_all() but let's be explicit for CPU usage 
    // measurement if this was a long-running instance.
    sys.refresh_all();
    
    let total_mem = sys.total_memory(); // in Bytes
    let used_mem = sys.used_memory();   // in Bytes
    let mem_percentage = if total_mem > 0 {
        (used_mem as f64 / total_mem as f64) * 100.0
    } else {
        0.0
    };
    
    let uptime = System::uptime(); // Static method in 0.38
    let uptime_days = uptime / (24 * 3600);
    let uptime_hours = (uptime % (24 * 3600)) / 3600;
    let uptime_mins = (uptime % 3600) / 60;

    let os_name = System::name().unwrap_or_else(|| "Unknown OS".to_string());
    let os_version = System::os_version().unwrap_or_default();
    let kernel_version = System::kernel_version().unwrap_or_default();

    // Get current process info
    let pid = std::process::id();
    let process_stats = if let Some(process) = sys.process(Pid::from_u32(pid)) {
        let p_mem = process.memory(); // in Bytes
        let p_cpu = process.cpu_usage();
        format!(
            "Process:\n  CPU Usage: {:.1}%\n  RAM Usage: {}\n",
            p_cpu, format_bytes(p_mem)
        )
    } else {
        "Process info unavailable\n".to_string()
    };

    let bar = create_progress_bar(used_mem, total_mem, 10);

    let message = format!(
        "📊 <b>Bot Statistics</b>\n\n\
        <code>System:\n  OS: {} {} ({})\n  Uptime: {}d {}h {}m\n\n\
        Resources:\n  Total RAM: {}\n  Used RAM:  {} {} {:.1}%\n\n\
        {}</code>",
        os_name, os_version, kernel_version,
        uptime_days, uptime_hours, uptime_mins,
        format_bytes(total_mem),
        format_bytes(used_mem),
        bar,
        mem_percentage,
        process_stats
    );

    bot.send_message(msg.chat.id, message)
        .parse_mode(teloxide::types::ParseMode::Html)
        .reply_parameters(ReplyParameters::new(msg.id))
        .await?;

    Ok(())
}
