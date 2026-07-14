use anyhow::Result;
use serde::de::DeserializeOwned;

use super::{Client, response};

pub(crate) const PAGE_SIZE: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Page {
    pub(crate) number: u32,
    pub(crate) per_page: u32,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            number: 1,
            per_page: PAGE_SIZE,
        }
    }
}

impl Page {
    fn next(self) -> Self {
        Self {
            number: self.number + 1,
            ..self
        }
    }
}

pub(crate) async fn collect_paginated<T, F>(client: &Client, mut path_for_page: F) -> Result<Vec<T>>
where
    T: DeserializeOwned,
    F: FnMut(Page) -> String,
{
    let mut page = Page::default();
    let mut items = Vec::new();

    loop {
        let path = path_for_page(page);
        let page_items: Vec<T> =
            response::expect_json(client.get(&path).await?, "GET", &path).await?;
        let item_count = page_items.len();
        items.extend(page_items);

        if item_count < page.per_page as usize {
            break;
        }

        page = page.next();
    }

    Ok(items)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::github::Client;

    use super::collect_paginated;

    #[tokio::test]
    async fn paginated_collection_preserves_safe_classified_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/orgs/test-org/repos"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(422).set_body_json(json!({
                "message": "Validation Failed",
                "errors": [{
                    "resource": "Repository",
                    "field": "name",
                    "code": "invalid",
                    "message": "secret-name"
                }],
                "secret": "do-not-log"
            })))
            .mount(&server)
            .await;

        let client = Client::new_for_test("test-org", &server.uri());
        let error = collect_paginated::<serde_json::Value, _>(&client, |page| {
            format!(
                "/orgs/test-org/repos?per_page={}&page={}",
                page.per_page, page.number
            )
        })
        .await
        .expect_err("validation responses should propagate as errors");

        let display = error.to_string();
        assert!(display.contains("Validation Failed"));
        assert!(display.contains("Repository.name (invalid)"));
        assert!(display.contains("response body omitted"));
        assert!(!display.contains("secret-name"));
        assert!(!display.contains("do-not-log"));
    }
}
