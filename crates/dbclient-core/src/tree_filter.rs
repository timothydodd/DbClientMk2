//! Schema-tree filter syntax: spaces between words are AND, `|` separates alternatives (OR).
//! `customer order|client name` matches names containing (customer AND order) OR (client AND name).

pub fn parse(raw: &str) -> Vec<Vec<String>> {
    raw.split('|')
        .map(|alt| alt.split(|c: char| c == ' ' || c == '\t').filter(|t| !t.is_empty()).map(|t| t.to_lowercase()).collect::<Vec<_>>())
        .filter(|alt: &Vec<String>| !alt.is_empty())
        .collect()
}

pub fn matches(name: &str, alternatives: &[Vec<String>]) -> bool {
    let lower = name.to_lowercase();
    alternatives.iter().any(|alt| alt.iter().all(|t| lower.contains(t.as_str())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn and_or_semantics() {
        let f = parse("customer order|client name");
        assert!(matches("CustomerOrders", &f));
        assert!(matches("client_name", &f));
        assert!(!matches("customers", &f));
        assert!(parse(" | ").is_empty());
    }
}
