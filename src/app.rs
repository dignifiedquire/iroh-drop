use std::collections::HashMap;

use js_sys::Uint8Array;
use leptos::html::Div;
use leptos::leptos_dom::ev::SubmitEvent;
use leptos::*;
use leptos_use::{use_drop_zone_with_options, UseDropZoneEvent, UseDropZoneOptions, UseDropZoneReturn};
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

    let discover = move |ev: SubmitEvent| {
        ev.prevent_default();
        spawn_local(async move {
            let result = invoke_without_args("discover").await;
            let discover: Vec<(String, String)> = serde_wasm_bindgen::from_value(result).unwrap();
            set_discover_msg.update(|val| {
                for (name, node_id) in discover {
                    val.insert(name, node_id);
                }
            });
        });
    };
    spawn_local(async move {
        let unlisten = listen::<(String, String), _>("discovery", move |(name, node_id)| {
            logging::log!("recv event: {:?}", node_id);
            set_discover_msg.update(|val| {
                val.insert(name, node_id);
            });
        })
        .await;

        on_cleanup(unlisten);
    });

    view! {
        <main class="container">
            <p>"Discover local iroh nodes."</p>

            <form class="row" on:submit=discover>
                <button type="submit">"Discover"</button>
            </form>

        <p><b>{ move || discover_msg.get().into_iter().map(|(name, node_id)| {
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
            let buffer = JsFuture::from(file.array_buffer()).await.expect("failed future");
            let array = Uint8Array::new(&buffer);
            let file_data: Vec<u8> = array.to_vec();
            logging::log!("converting args");
            let args = serde_wasm_bindgen::to_value(&SendFileArgs {
                node_id,
                file_name: file.name(),
                file_data,
            }).expect("failed conversion");
            logging::log!("args {:?}", args);
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

    view! {
        <div node_ref=drop_zone_el class="dropzone">
          <p>
           {name}: {node_id}
          </p>
          <p>"Drop files here"</p>
          <p>"Is over drop zone:" {move || is_over_drop_zone.get()}</p>
          <p>"Dropped:" {move || dropped.get()}</p>
          <p>
            "Dropped Files:"
            { move || files.get().iter().map(|f| format!("{}: {}bytes", f.name(), f.size())).collect::<Vec<_>>()}
          </p>
        </div>
    }
}
