use xcap::Monitor;

fn main() -> anyhow::Result<()> {
    println!("Monitor Information\n");

    let monitors = Monitor::all()?;

    println!("Found {} monitor(s):\n", monitors.len());

    for (i, monitor) in monitors.iter().enumerate() {
        println!("Monitor {}:", i);
        println!("  Name:       {}", monitor.name().unwrap_or_default());
        println!("  ID:         {}", monitor.id().unwrap_or(0));
        println!(
            "  Resolution: {}x{}",
            monitor.width().unwrap_or(0),
            monitor.height().unwrap_or(0)
        );
        println!(
            "  Position:   ({}, {})",
            monitor.x().unwrap_or(0),
            monitor.y().unwrap_or(0)
        );
        println!("  Is Primary: {}", monitor.is_primary().unwrap_or(false));
        println!();
    }

    Ok(())
}
