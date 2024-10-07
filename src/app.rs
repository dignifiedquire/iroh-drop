use std::collections::HashMap;

use js_sys::Uint8Array;
use leptoaster::*;
use leptos::html::Div;
use leptos::leptos_dom::ev::SubmitEvent;
use leptos::*;
use leptos_use::{
    use_drop_zone_with_options, UseDropZoneEvent, UseDropZoneOptions, UseDropZoneReturn,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
    async fn invoke(cmd: &str, args: JsValue) -> JsValue;
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    async fn invoke_without_args(cmd: &str) -> JsValue;
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = "listen")]
    async fn listen_sys(event: &str, handler: &js_sys::Function) -> js_sys::Function;
}

#[derive(Serialize, Deserialize)]
pub struct Event<T> {
    pub event: String,
    pub payload: T,
    pub id: f64,
}

async fn listen<T: DeserializeOwned, F: Fn(T) + 'static>(event: &str, handler: F) -> impl FnOnce() {
    logging::log!("listenting to event: {}", event);
    let closure = Closure::<dyn FnMut(_)>::new(move |s: JsValue| {
        let event: Event<T> = serde_wasm_bindgen::from_value(s).unwrap();
        handler(event.payload);
    });

    let unlisten = listen_sys(event, closure.as_ref().unchecked_ref()).await;
    closure.forget();

    move || {
        logging::log!("unlistening");
        unlisten.call0(&JsValue::NULL).expect("failed to unlisten");
    }
}

#[component]
pub fn App() -> impl IntoView {
    let (discover_msg, set_discover_msg) = create_signal(HashMap::new());

    let (my_node_id, set_my_node_id) = create_signal(String::new());

    provide_toaster();

    spawn_local(async move {
        let result = invoke_without_args("node_id").await;
        let my_node_id: String = serde_wasm_bindgen::from_value(result).unwrap();
        set_my_node_id.set(my_node_id);
    });

    let discover = move |ev: SubmitEvent| {
        ev.prevent_default();
        spawn_local(async move {
            let result = invoke_without_args("discover").await;
            let discover: Vec<(String, String)> = serde_wasm_bindgen::from_value(result).unwrap();
            logging::log!("discovered: {:?}", discover);
            set_discover_msg.update(|val| {
                for (name, node_id) in discover {
                    val.insert(node_id, name);
                }
            });
        });
    };
    spawn_local(async move {
        let unlisten = listen::<(String, String), _>("discovery", move |(name, node_id)| {
            logging::log!("recv event: {}: {}", name, node_id);
            set_discover_msg.update(|val| {
                val.insert(node_id, name);
            });
        })
        .await;

        on_cleanup(unlisten);
    });

    let toaster = expect_toaster();
    spawn_local(async move {
        let unlisten =
            listen::<(String, String, u64), _>("file-downloaded", move |(name, hash, size)| {
                logging::log!("recv event file-downloaed: {} - {} - {}", name, hash, size);
                toaster.toast(
                    ToastBuilder::new(&format!("File received: {} ({}bytes)", name, size))
                        .with_level(ToastLevel::Success)
                        .with_expiry(None)
                        .with_position(ToastPosition::TopRight),
                );
            })
            .await;

        on_cleanup(unlisten);
    });

    view! {
        <Toaster stacked={true} />

        <main class="container">
            <p>"Discover local iroh nodes."</p>
            <p>"My Node: " { move || my_node_id.get() }</p>

            <form class="row" on:submit=discover>
                <button type="submit">"Discover"</button>
            </form>

        <p><b>{ move || discover_msg.get().into_iter().map(|(node_id, name)| {
            node_view(name, node_id)
            }).collect_view() }</b></p>
        </main>
    }
}

fn node_view(name: String, node_id: String) -> impl IntoView {
    let (dropped, set_dropped) = create_signal(false);

    let drop_zone_el = create_node_ref::<Div>();

    #[derive(Debug, Serialize, Deserialize)]
    struct SendFileArgs {
        node_id: String,
        file_name: String,
        file_data: Vec<u8>,
    }

    let node = node_id.clone();
    let on_drop = move |event: UseDropZoneEvent| {
        let node_id = node.clone();
        set_dropped.set(true);
        spawn_local(async move {
            let file = &event.files[0];
            logging::log!("reading: {:?}", file);
            let buffer = JsFuture::from(file.array_buffer())
                .await
                .expect("failed future");
            let array = Uint8Array::new(&buffer);
            let file_data: Vec<u8> = array.to_vec();
            logging::log!("sending file to {}", node_id);
            let args = serde_wasm_bindgen::to_value(&SendFileArgs {
                node_id,
                file_name: file.name(),
                file_data,
            })
                .expect("failed conversion");
            let result = invoke("send_file", args).await;
            logging::log!("sent file {:?}", result);
        })
    };

    let UseDropZoneReturn {
        is_over_drop_zone,
        files,
    } = use_drop_zone_with_options(
        drop_zone_el,
        UseDropZoneOptions::default()
            .on_drop(on_drop)
            .on_enter(move |_| set_dropped.set(false)),
    );

    let class = move || {
        let mut base = "row dropzone".to_string();
        if is_over_drop_zone.get() {
            base += " dropping";
        }
        base
    };
    logging::log!("showing {}: {}", name, node_id);

    view! {
        <div node_ref=drop_zone_el class={ class }>
          <p>
            {format!("{} ({})", name, node_id)}
          </p>
        </div>
    }
}
