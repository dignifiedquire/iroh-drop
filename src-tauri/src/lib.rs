use futures_lite::stream::StreamExt;
use iroh::net::discovery::local_swarm_discovery::NAME as SWARM_DISCOVERY_NAME;
use tauri::Emitter;

#[tauri::command]
async fn discover(state: tauri::State<'_, iroh::node::MemNode>) -> Result<Vec<String>, ()> {
    use iroh::net::endpoint::Source;

    let limit = std::time::Duration::from_secs(60);
    let mut eps = Vec::new();

    for remote in state.endpoint().remote_info_iter() {
        for (source, last_seen) in remote.sources() {
            if let Source::Discovery { name } = source {
                if name == SWARM_DISCOVERY_NAME && last_seen <= limit {
                    eps.push(remote.node_id.to_string());
                }
            }
        }
    }
    println!("found {} nodes", eps.len());

    Ok(eps)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub async fn run() {
    let iroh_node = iroh::node::Node::memory()
        .node_discovery(iroh::node::DiscoveryConfig::Default)
        .spawn()
        .await
        .expect("failed to start iroh");

    let endpoint = iroh_node.endpoint().clone();

    println!("starting iroh");
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(not(mobile))]
            {
                tauri::WebviewWindowBuilder::new(
                    app,
                    "main",
                    tauri::WebviewUrl::App("index.html".into()),
                )
                .inner_size(800., 600.)
                .title("iroh-drop")
                .disable_drag_drop_handler()
                .build()?;
            }
            #[cfg(mobile)]
            {
                tauri::WebviewWindowBuilder::new(
                    app,
                    "main",
                    tauri::WebviewUrl::App("index.html".into()),
                )
                .build()?;
            }

            let handle = app.handle().clone();
            tokio::task::spawn(async move {
                let mut stream = endpoint.discovery().unwrap().subscribe().unwrap();
                while let Some(item) = stream.next().await {
                    if item.provenance == SWARM_DISCOVERY_NAME {
                        handle.emit("discovery", item.node_id.to_string()).ok();
                    }
                }
            });

            Ok(())
        })
        .plugin(tauri_plugin_shell::init())
        .manage(iroh_node)
        .invoke_handler(tauri::generate_handler![discover])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
