#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn builder() -> HttpRequestBuilder {
        HttpRequestBuilder::default()
    }
}

#[derive(Debug, Default, Clone)]
pub struct HttpRequestBuilder {
    method: Option<String>,
    url: Option<String>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpRequestBuilder {
    pub fn method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.headers = headers;
        self
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn build(self) -> Result<HttpRequest, HttpRequestBuildError> {
        Ok(HttpRequest {
            method: self.method.ok_or(HttpRequestBuildError::MissingMethod)?,
            url: self.url.ok_or(HttpRequestBuildError::MissingUrl)?,
            headers: self.headers,
            body: self.body,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpRequestBuildError {
    MissingMethod,
    MissingUrl,
}
