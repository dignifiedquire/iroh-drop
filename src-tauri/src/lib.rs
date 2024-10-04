use std::sync::Arc;

use futures_lite::stream::StreamExt;
use iroh::net::{discovery::local_swarm_discovery::NAME as SWARM_DISCOVERY_NAME, NodeId};
use log::info;
use tauri::Emitter;
use tauri_plugin_log::{Target, TargetKind};

mod protocol;

#[tauri::command(rename_all = "snake_case")]
async fn send_file(
    proto: tauri::State<'_, Arc<protocol::Protocol>>,
    node_id: String,
    file_name: String,
    file_data: Vec<u8>,
) -> Result<(), ()> {
    let node_id: NodeId = node_id.parse().map_err(|_| ())?;
    proto.send_file(node_id, file_name, file_data).await.map_err(|_| ())?;

    Ok(())
}

#[tauri::command]
async fn discover(
    iroh: tauri::State<'_, iroh::node::MemNode>,
    proto: tauri::State<'_, Arc<protocol::Protocol>>,
) -> Result<Vec<(String, String)>, ()> {
    use iroh::net::endpoint::Source;

    let limit = std::time::Duration::from_secs(60);
    let mut eps = Vec::new();

    for remote in iroh.endpoint().remote_info_iter() {
        for (source, last_seen) in remote.sources() {
            if let Source::Discovery { name } = source {
                if name == SWARM_DISCOVERY_NAME && last_seen <= limit {
                    let name = proto.send_intro(remote.node_id).await.map_err(|_| ())?;
                    eps.push((name, remote.node_id.to_string()));
                }
            }
        }
    }

    println!("found {} nodes", eps.len());

    Ok(eps)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (iroh_node, proto) = tauri::async_runtime::block_on(async move {
        info!("starting iroh");
        let builder = iroh::node::Node::memory()
            .node_discovery(iroh::node::DiscoveryConfig::Default)
            .build()
            .await
            .expect("failed to build iroh");

        let proto = protocol::Protocol::new(
            "drop-1".to_string(),
            builder.client().clone(),
            builder.endpoint().clone(),
        );
        let node = builder
            .accept(protocol::ALPN.to_vec(), proto.clone())
            .spawn()
            .await
            .expect("failed to spawn iroh");
        (node, proto)
    });

    info!("inner run");
    let endpoint = iroh_node.endpoint().clone();
    let protocol = proto.clone();

    tauri::Builder::default()
        .setup(|app| {
            info!("setup");

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

            tauri::async_runtime::spawn(async move {
                info!("spawning discovery stream");
                let mut stream = endpoint.discovery().unwrap().subscribe().unwrap();
                while let Some(item) = stream.next().await {
                    if item.provenance == SWARM_DISCOVERY_NAME {
                        if !proto.is_known_node(&item.node_id).await {
                            match proto.send_intro(item.node_id).await {
                                Ok(name) => {
                                    let name = format!("{} ({})", name, item.node_id);
                                    handle.emit("discovery", (name, item.node_id.to_string())).ok();
                                }
                                Err(err) => {
                                    eprintln!("failed to discover: {:?}", err);
                                }
                            }
                        }
                    }
                }
            });

            Ok(())
        })
        .plugin(tauri_plugin_shell::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                    Target::new(TargetKind::Webview),
                ])
                .build(),
        )
        .manage(iroh_node)
        .manage(protocol)
        .invoke_handler(tauri::generate_handler![discover, send_file])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
