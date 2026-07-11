// Example: P2 taxonomy — classify errors deterministically
use witslog_core::{error, warn, Classifier};

fn main() {
    let classifier = Classifier::built_in();

    // Error 1: Network timeout
    let event1 = error("web-service", "API call timed out")
        .error_code("ETIMEDOUT")
        .classify(&classifier)
        .build();

    println!("Event 1: {}", event1.message);
    println!("  Category: {:?}", event1.category);
    println!("  Tags: {:?}\n", event1.tags);

    // Error 2: DNS resolution failure
    let event2 = error("db-client", "Could not resolve hostname")
        .exception("ENOTFOUND")
        .classify(&classifier)
        .build();

    println!("Event 2: {}", event2.message);
    println!("  Category: {:?}", event2.category);
    println!("  Tags: {:?}\n", event2.tags);

    // Error 3: Disk full (message keyword match)
    let event3 = warn("backup-service", "ERROR: Disk full, cannot write")
        .classify(&classifier)
        .build();

    println!("Event 3: {}", event3.message);
    println!("  Category: {:?}", event3.category);
    println!("  Tags: {:?}\n", event3.tags);

    // Error 4: Unclassified (no rule match)
    let event4 = error("unknown-service", "Something weird happened")
        .classify(&classifier)
        .build();

    println!("Event 4: {}", event4.message);
    println!("  Category: {:?}", event4.category);
    println!("  Tags: {:?}", event4.tags);

    println!("\n✓ All events classified deterministically");
}
