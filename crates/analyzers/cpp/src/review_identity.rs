//! Revision matching hints; display names and call-resolution identities stay intact.
use codegraide_core::ProjectSymbol;

pub fn cpp_review_identity(symbol: &ProjectSymbol) -> String {
    let name = &symbol.id.qualified_name;
    let file_owner = format!(
        "@file::{}::initialization",
        symbol.path.to_string_lossy().replace(['\\', '/'], "::")
    );
    if name == &file_owner {
        return "@file::initialization".into();
    }
    let marker = "<anonymous-namespace>@";
    let mut rest = name.as_str();
    let mut result = String::new();
    while let Some(start) = rest.find(marker) {
        result.push_str(&rest[..start]);
        rest = &rest[start + marker.len()..];
        let line = rest.bytes().take_while(u8::is_ascii_digit).count();
        let column = rest
            .get(line + 1..)
            .unwrap_or_default()
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if line > 0 && rest.as_bytes().get(line) == Some(&b':') && column > 0 {
            result.push_str("<anonymous-namespace>");
            rest = &rest[line + 1 + column..];
        } else {
            result.push_str(marker);
        }
    }
    result.push_str(rest);
    result
}
