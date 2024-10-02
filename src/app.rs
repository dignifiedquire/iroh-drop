use std::collections::HashSet;

use leptos::html::Div;
use leptos::leptos_dom::ev::SubmitEvent;
use leptos::*;
use leptos_use::{use_drop_zone_with_options, UseDropZoneOptions, UseDropZoneReturn};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use wasm_bindgen::prelude::*;

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
    let (discover_msg, set_discover_msg) = create_signal(HashSet::new());

    let discover = move |ev: SubmitEvent| {
        ev.prevent_default();
        spawn_local(async move {
            let result = invoke_without_args("discover").await;
            let discover: Vec<String> = serde_wasm_bindgen::from_value(result).unwrap();
            set_discover_msg.update(|val| {
                for el in discover {
                    val.insert(el);
                }
            });
        });
    };
    spawn_local(async move {
        let unlisten = listen::<String, _>("discovery", move |node_id| {
            logging::log!("recv event: {:?}", node_id);
            set_discover_msg.update(|val| {
                val.insert(node_id);
            });
        })
        .await;

        on_cleanup(unlisten);
    });

    let (dropped, set_dropped) = create_signal(false);

    let drop_zone_el = create_node_ref::<Div>();

    let UseDropZoneReturn {
        is_over_drop_zone,
        files,
    } = use_drop_zone_with_options(
        drop_zone_el,
        UseDropZoneOptions::default()
            .on_drop(move |_| set_dropped.set(true))
            .on_enter(move |_| set_dropped.set(false)),
    );

    view! {
        <main class="container">
            <p>"Discover local iroh nodes."</p>

            <form class="row" on:submit=discover>
                <button type="submit">"Discover"</button>
            </form>

            <p><b>{ move || discover_msg.get().into_iter().map(|v| {
                  view! {
                    <p>{v}</p>
                  }
            }).collect_view() }</b></p>
        </main>
    }
}

// <div node_ref=drop_zone_el class="dropzone">
// <p>
// "Drop files here"
// </p>
// <p>"Is over drop zone:" {move || is_over_drop_zone.get()}</p>
// <p>"Dropped:" {move || dropped.get()}</p>
// <p>"Dropped Files:" {move || files.get().iter().map(|f| format!("{}: {}bytes", f.name(), f.size())).collect::<Vec<_>>()}</p>
// <p></p>
// </div>
