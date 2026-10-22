use sm::{
    analysis::{AgentRunner, CliRunner},
    model::Client,
};
#[test]
#[ignore = "Requires existing Codex/Kimi credentials and contacts the configured model services"]
fn native_cli_transports_return_typed_empty_candidates_without_business_tools() {
    for client in [Client::Codex, Client::Kimi] {
        let runner = CliRunner::new(client, client.to_string().into());
        let reply=runner.run("This is a transport verification with no task records or resources. Do not invoke any skill or tool. Return exactly the JSON object {\"candidates\":[]}.").unwrap();
        assert!(reply.candidates.is_empty());
        println!("{client}: typed empty candidate response passed; no business task executed");
    }
}
