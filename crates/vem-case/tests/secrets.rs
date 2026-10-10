use vem_case::secrets::scan;

fn rules(text: &str) -> Vec<&'static str> {
    scan(text).into_iter().map(|m| m.rule).collect()
}

#[test]
fn each_rule_has_a_positive() {
    let cases: &[(&str, String)] = &[
        (
            "aws-access-key-id",
            format!("key {}", concat!("AKIA", "IOSFODNN7EXAMPLE")),
        ),
        (
            "github-token",
            format!(
                "t {}",
                concat!("ghp_", "0123456789abcdefghijklmnopqrstuvwxyzAB")
            ),
        ),
        (
            "github-token",
            format!(
                "t {}",
                concat!("github_pat_", "11ABCDEFG0123456789_abcdefghij")
            ),
        ),
        (
            "gitlab-token",
            concat!("glpat-", "abcdefghij0123456789").to_string(),
        ),
        (
            "slack-token",
            concat!("xoxb-", "123456789012-abcdef").to_string(),
        ),
        (
            "anthropic-key",
            concat!("sk-ant-", "api03-abcdefghijklmnopqrstuvwxyz").to_string(),
        ),
        (
            "openai-key",
            concat!("sk-", "proj-abcdefghijklmnopqrstuvwxyz").to_string(),
        ),
        (
            "google-api-key",
            concat!("AIza", "SyA-abcdefghijklmnopqrstuvwxyz012345").to_string(),
        ),
        (
            "private-key",
            concat!("-----BEGIN OPENSSH ", "PRIVATE KEY-----").to_string(),
        ),
        (
            "jwt",
            concat!(
                "eyJhbGciOiJIUzI1NiJ9",
                ".",
                "eyJzdWIiOiIxMjM0NTY3ODkwIn0",
                ".",
                "abcdefghijk"
            )
            .to_string(),
        ),
        (
            "generic-assignment",
            "password = hunter2hunter2".to_string(),
        ),
    ];
    for (rule, text) in cases {
        assert_eq!(rules(text), vec![*rule], "{text}");
    }
}

#[test]
fn each_rule_has_a_negative() {
    for text in [
        "AKIA123",   // too short
        "ghp_short", // too short
        "glpat-short",
        "xoxb-1",
        "sk-ant-short",
        "sk-short",
        "AIzaShort",
        "-----BEGIN PUBLIC KEY-----",
        "eyJhbGciOiJIUzI1NiJ9.notjwt",
        "password = short",
        "the token was revoked",
    ] {
        assert!(scan(text).is_empty(), "{text}: {:?}", scan(text));
    }
}

#[test]
fn anthropic_keys_are_not_also_openai_keys_and_generic_yields_to_specific() {
    let key = concat!("sk-ant-", "api03-abcdefghijklmnopqrstuvwxyz");
    assert_eq!(rules(&format!("api_key: {key}")), vec!["anthropic-key"]);
    let gh = concat!("ghp_", "0123456789abcdefghijklmnopqrstuvwxyzAB");
    let m = scan(&format!("export GITHUB_TOKEN={gh} && gh auth status"));
    assert_eq!(m.len(), 1);
    assert_eq!(
        (m[0].rule, m[0].confidence, m[0].matched.as_str()),
        ("github-token", "high", gh)
    );
    assert_eq!(m[0].offset, "export GITHUB_TOKEN=".len());
    assert_eq!(m[0].length, gh.len());
}

#[test]
fn matches_are_sorted_and_carry_confidence() {
    let text = format!(
        "{} then password: correcthorsebattery",
        concat!("AKIA", "IOSFODNN7EXAMPLE")
    );
    let m = scan(&text);
    assert_eq!(
        m.iter().map(|x| (x.rule, x.confidence)).collect::<Vec<_>>(),
        vec![("aws-access-key-id", "high"), ("generic-assignment", "low")]
    );
    assert!(m[0].offset < m[1].offset);
}
