fn main() {
    #[cfg(windows)]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("assets/agentbell.ico");
        resource.set("ProductName", "AgentBell");
        resource.set("FileDescription", "AgentBell 局域网 Agent 完成通知");
        resource.set("LegalCopyright", "AgentBell contributors");
        resource
            .compile()
            .expect("failed to compile Windows resources");
    }
}
