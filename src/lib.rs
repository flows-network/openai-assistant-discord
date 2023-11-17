use async_openai_wasi::{
    types::{
        CreateMessageRequestArgs, CreateRunRequestArgs, CreateThreadRequestArgs, MessageContent,
        RunStatus,
    },
    Client,
};
use discord_flows::{
    application_command_handler,
    http::HttpBuilder,
    message_handler,
    model::{
        prelude::application::interaction::application_command::ApplicationCommandInteraction,
        Message,
    },
    Bot, ProvidedBot,
};
use dotenv::dotenv;
use flowsnet_platform_sdk::logger;
use serde_json::Value;
use std::env;

#[no_mangle]
#[tokio::main(flavor = "current_thread")]
pub async fn on_deploy() {
    logger::init();
    dotenv().ok();

    let discord_token = env::var("discord_token").unwrap();
    let bot = ProvidedBot::new(&discord_token);

    register_commands().await;

    bot.listen_to_messages().await;

    bot.listen_to_application_commands().await;
}

#[message_handler]
async fn handle(msg: Message) {
    logger::init();
    let discord_token = std::env::var("discord_token").unwrap();
    let bot = ProvidedBot::new(&discord_token);

    if msg.author.bot {
        return;
    }
    let client = bot.get_client();
    handle_inner(msg, client).await;
}

async fn handle_inner(msg: Message, client: discord_flows::http::Http) {
    let channel_id = msg.channel_id.to_string();

    let thread_id = match store_flows::get(&channel_id) {
        Some(ti) => ti.as_str().unwrap().to_owned(),
        None => {
            let ti = create_thread().await;

            store_flows::set(&channel_id, serde_json::Value::String(ti.clone()), None);
            ti
        }
    };

    let response = run_message(thread_id.as_str(), msg.content).await;
    _ = client
        .send_message(
            msg.channel_id.into(),
            &serde_json::json!({
                "content": response,
            }),
        )
        .await;
}

#[application_command_handler]
async fn handler(ac: ApplicationCommandInteraction) {
    logger::init();
    let discord_token = env::var("discord_token").unwrap();
    let bot = ProvidedBot::new(discord_token);
    let client = bot.get_client();

    client.set_application_id(ac.application_id.into());

    let _ = respond_to_ac(ac, client).await;
}

async fn respond_to_ac(ac: ApplicationCommandInteraction, client: discord_flows::http::Http) {
    match ac.data.name.as_str() {
        "restart" => {
            let channel_id = ac.channel_id.to_string();
            if let Some(ti) = store_flows::get(&channel_id) {
                delete_thread(ti.as_str().unwrap()).await;
                store_flows::del(&channel_id);
                return;
                _ = client
                    .create_interaction_response(
                        ac.id.into(),
                        &ac.token,
                        &serde_json::json!({
                            "content": "thread deleted",
                        }),
                    )
                    .await;
                return;
            }
        }

        _ => {}
    }
}

async fn create_thread() -> String {
    let client = Client::new();

    let create_thread_request = CreateThreadRequestArgs::default().build().unwrap();

    match client.threads().create(create_thread_request).await {
        Ok(to) => {
            log::info!("New thread (ID: {}) created.", to.id);
            to.id
        }
        Err(e) => {
            panic!("Failed to create thread. {:?}", e);
        }
    }
}

async fn delete_thread(thread_id: &str) {
    let client = Client::new();

    match client.threads().delete(thread_id).await {
        Ok(_) => {
            log::info!("Old thread (ID: {}) deleted.", thread_id);
        }
        Err(e) => {
            log::error!("Failed to delete thread. {:?}", e);
        }
    }
}

async fn run_message(thread_id: &str, text: String) -> String {
    let client = Client::new();
    let assistant_id = std::env::var("ASSISTANT_ID").unwrap();

    let mut create_message_request = CreateMessageRequestArgs::default().build().unwrap();
    create_message_request.content = text;
    client
        .threads()
        .messages(&thread_id)
        .create(create_message_request)
        .await
        .unwrap();

    let mut create_run_request = CreateRunRequestArgs::default().build().unwrap();
    create_run_request.assistant_id = assistant_id;
    let run_id = client
        .threads()
        .runs(&thread_id)
        .create(create_run_request)
        .await
        .unwrap()
        .id;
    log::info!("Run created {}", run_id);

    let mut result = Some("Timeout");
    for _ in 0..5 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let run_object = client
            .threads()
            .runs(&thread_id)
            .retrieve(run_id.as_str())
            .await
            .unwrap();
        result = match run_object.status {
            RunStatus::Queued | RunStatus::InProgress | RunStatus::Cancelling => {
                continue;
            }
            RunStatus::RequiresAction => Some("Action required for OpenAI assistant"),
            RunStatus::Cancelled => Some("Run is cancelled"),
            RunStatus::Failed => Some("Run is failed"),
            RunStatus::Expired => Some("Run is expired"),
            RunStatus::Completed => None,
        };
        break;
    }

    match result {
        Some(r) => String::from(r),
        None => {
            let mut thread_messages = client
                .threads()
                .messages(&thread_id)
                .list(&[("limit", "1")])
                .await
                .unwrap();

            let c = thread_messages.data.pop().unwrap();
            let c = c.content.into_iter().filter_map(|x| match x {
                MessageContent::Text(t) => Some(t.text.value),
                _ => None,
            });

            c.collect()
        }
    }
}

async fn register_commands() {
    let bot_id = env::var("bot_id").unwrap_or("1124137839601406013".to_string());

    let command = serde_json::json!({
        "name": "restart",
        "description": "Delete generated messages",
    });

    let discord_token = env::var("discord_token").unwrap();
    let http_client = HttpBuilder::new(discord_token)
        .application_id(bot_id.parse().unwrap())
        .build();

    match http_client
        .create_global_application_command(&command)
        .await
    {
        Ok(_) => log::info!("Successfully registered command"),
        Err(err) => log::error!("Error registering command: {}", err),
    }
}
