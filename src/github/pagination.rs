use crate::github::error::ReadContext;
use anyhow::Result;
use serde::de::DeserializeOwned;

use super::actions::{ReadOutcome, classify_read};
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
    pub(crate) fn with_size(per_page: u32) -> Self {
        Self {
            number: 1,
            per_page,
        }
    }

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

/// Collect every page of a list endpoint, classifying each response so an
/// expected per-endpoint condition becomes a [`ReadOutcome`] instead of an error.
pub(crate) async fn collect_paginated_checked<T, F>(
    client: &Client,
    mut path_for_page: F,
) -> Result<ReadOutcome<Vec<T>>>
where
    T: DeserializeOwned,
    F: FnMut(Page) -> String,
{
    let mut page = Page::default();
    let mut items = Vec::new();

    loop {
        let path = path_for_page(page);
        let page_items: Vec<T> = match classify_read(client.get(&path).await?, "GET", &path, false)
            .await?
        {
            ReadOutcome::Available(values) => values,
            ReadOutcome::NotApplicable(reason) => return Ok(ReadOutcome::NotApplicable(reason)),
            ReadOutcome::PermissionDenied(reason) => {
                return Ok(ReadOutcome::PermissionDenied(reason));
            }
            ReadOutcome::Unavailable(reason) => return Ok(ReadOutcome::Unavailable(reason)),
        };
        let count = page_items.len();
        items.extend(page_items);
        if count < page.per_page as usize {
            break;
        }
        page = page.next();
    }

    Ok(ReadOutcome::Available(items))
}

/// One page of a wrapped list response such as `{ "total_count": 2, "workflows": [...] }`.
pub(crate) struct WrappedPage<T> {
    pub(crate) items: Vec<T>,
    /// Reported total, when the endpoint provides one. Collection stops once it is reached.
    pub(crate) total_count: Option<usize>,
}

/// Collect every page of an endpoint whose pages wrap the items in an object.
///
/// `unwrap` extracts the items from each decoded page. Errors name `what`, for example "Actions variables".
pub(crate) async fn collect_paginated_wrapped<W, T, F, U>(
    client: &Client,
    page_size: u32,
    what: &'static str,
    mut path_for_page: F,
    mut unwrap: U,
) -> Result<Vec<T>>
where
    W: DeserializeOwned,
    F: FnMut(Page) -> String,
    U: FnMut(W) -> WrappedPage<T>,
{
    let mut page = Page::with_size(page_size);
    let mut items = Vec::new();

    loop {
        let path = path_for_page(page);
        let body: W = response::expect_json(client.get(&path).await?, "GET", &path)
            .await
            .read_ctx(what)?;
        let wrapped = unwrap(body);
        let count = wrapped.items.len();
        items.extend(wrapped.items);

        if count < page.per_page as usize
            || wrapped
                .total_count
                .is_some_and(|total_count| items.len() >= total_count)
        {
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

    use super::{WrappedPage, collect_paginated, collect_paginated_wrapped};

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
        assert!(!display.contains("body omitted"));
        assert!(display.contains("secret-name"));
        assert!(!display.contains("do-not-log"));
    }

    #[tokio::test]
    async fn wrapped_collection_follows_pages_and_stops_at_total_count() {
        let server = MockServer::start().await;
        for (page, items) in [("1", json!([1, 2])), ("2", json!([3, 4]))] {
            Mock::given(method("GET"))
                .and(path("/things"))
                .and(query_param("page", page))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "total_count": 4,
                    "things": items
                })))
                .expect(1)
                .mount(&server)
                .await;
        }

        #[derive(serde::Deserialize)]
        struct Wrapped {
            total_count: usize,
            things: Vec<u32>,
        }

        let client = Client::new_for_test("test-org", &server.uri());
        let things = collect_paginated_wrapped(
            &client,
            2,
            "things",
            |page| format!("/things?per_page={}&page={}", page.per_page, page.number),
            |body: Wrapped| WrappedPage {
                items: body.things,
                total_count: Some(body.total_count),
            },
        )
        .await
        .unwrap();

        assert_eq!(things, vec![1, 2, 3, 4]);
    }
}
