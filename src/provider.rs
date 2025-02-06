use std::{borrow::Cow, collections::HashSet, fmt::Display, future::Future, io::Cursor, num::NonZeroU8};

use anni_flac::{
    blocks::BlockStreamInfo,
    prelude::{AsyncDecode, Encode},
};
use anni_provider::{AnniProvider, AudioInfo, AudioResourceReader, Range, ResourceReader};
use axum::http::{
    header::{AUTHORIZATION, CONTENT_RANGE, RANGE},
    Method,
};
use futures_util::StreamExt;
use reqwest_dav::{
    re_exports::reqwest::{self, Response},
    Auth, Client,
};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio_util::io::StreamReader;

pub struct WebdavProvider {
    client: Client,
}

impl WebdavProvider {
    pub fn new(host: String, auth: Auth) -> Self {
        Self {
            client: Client {
                agent: reqwest::Client::new(),
                host,
                auth,
                digest_auth: Default::default(),
            },
        }
    }
}

#[async_trait::async_trait]
impl AnniProvider for WebdavProvider {
    async fn albums(&self) -> anni_provider::Result<HashSet<Cow<str>>> {
        Ok(self
            .client
            .list_rsp("/", reqwest_dav::Depth::Number(1))
            .await
            .map_err(handle_dav_error)?
            .into_iter()
            .filter_map(|entry| {
                entry
                    .href
                    .trim_end_matches('/')
                    .rsplit_once('/')
                    .map(|(_, album_id)| Cow::Owned(album_id.to_owned()))
            })
            .collect())
    }

    async fn get_audio(
        &self,
        album_id: &str,
        disc_id: NonZeroU8,
        track_id: NonZeroU8,
        range: Range,
    ) -> anni_provider::Result<AudioResourceReader> {
        let path = format!("{album_id}/{disc_id}/{track_id}");
        let req = self
            .client
            .start_request(Method::GET, &path)
            .await
            .map_err(handle_dav_error)?;
        let req = match range.to_range_header() {
            Some(h) => req.header(RANGE, h),
            None => req,
        };
        let resp = req.send().await?;
        let content_length = resp.content_length().unwrap();
        let (duration, reader) = read_response(resp).await?;
        Ok(AudioResourceReader {
            info: AudioInfo {
                extension: String::from("flac"),
                size: content_length as usize,
                duration,
            },
            range,
            reader,
        })
    }

    async fn get_cover(
        &self,
        _album_id: &str,
        _disc_id: Option<NonZeroU8>,
    ) -> anni_provider::Result<ResourceReader> {
        todo!()
    }

    async fn reload(&mut self) -> anni_provider::Result<()> {
        Ok(())
    }
}

pub struct SeafileProvider {
    client: reqwest::Client,
    token: String,
    base: String,
    repo_id: String,
}

#[derive(Deserialize)]
struct DirectoryItem {
    pub name: String,
}

impl SeafileProvider {
    pub fn new(client: reqwest::Client, token: String, base: String, repo_id: String) -> Self {
        Self {
            client,
            token,
            base,
            repo_id,
        }
    }

    pub async fn list_albums(&self) -> reqwest::Result<Vec<String>> {
        let url = format!("{}/api2/repos/{}/dir/?t=d", self.base, self.repo_id);
        Ok(self
            .client
            .get(url)
            .header(AUTHORIZATION, format!("Token {}", self.token))
            .send()
            .await?
            .json::<Vec<DirectoryItem>>()
            .await?
            .into_iter()
            .map(|dir| dir.name)
            .collect())
    }

    pub async fn get_download_link(&self, path: impl Display) -> reqwest::Result<String> {
        let url = format!(
            "{server}/api2/repos/{repo_id}/file/?p={path}&reuse=1",
            server = self.base,
            repo_id = self.repo_id,
        );

        self.client
            .get(url)
            .header(AUTHORIZATION, format!("Token {}", self.token))
            .send()
            .await?
            .json()
            .await
    }
}

#[async_trait::async_trait]
impl AnniProvider for SeafileProvider {
    async fn albums(&self) -> anni_provider::Result<HashSet<Cow<str>>> {
        Ok(self
            .list_albums()
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    async fn get_audio(
        &self,
        album_id: &str,
        disc_id: NonZeroU8,
        track_id: NonZeroU8,
        range: Range,
    ) -> anni_provider::Result<AudioResourceReader> {
        let req = self.client.get(
            self.get_download_link(format!("{album_id}/{disc_id}/{track_id}.flac"))
                .await?,
        );
        let req = match range.to_range_header() {
            Some(h) => req.header(RANGE, h),
            None => req,
        };
        let resp = req.send().await?;
        let content_length = resp.content_length().unwrap();
        let (duration, reader) = read_response(resp).await?;
        Ok(AudioResourceReader {
            info: AudioInfo {
                extension: String::from("flac"),
                size: content_length as usize,
                duration,
            },
            range,
            reader,
        })
    }

    async fn get_cover(
        &self,
        _album_id: &str,
        _disc_id: Option<NonZeroU8>,
    ) -> anni_provider::Result<ResourceReader> {
        unimplemented!()
    }

    async fn reload(&mut self) -> anni_provider::Result<()> {
        Ok(())
    }
}

impl AnniURLProvider for SeafileProvider {
    async fn get_audio_link(
        &self,
        album_id: &str,
        disc_id: NonZeroU8,
        track_id: NonZeroU8,
        _range: Range,
    ) -> anni_provider::Result<Result<String, AudioResourceReader>> {
        Ok(Ok(self
            .get_download_link(format!("{album_id}/{disc_id}/{track_id}.flac"))
            .await?))
    }

    async fn get_cover_link(
        &self,
        album_id: &str,
        disc_id: Option<NonZeroU8>,
    ) -> anni_provider::Result<Result<String, ResourceReader>> {
        Ok(Ok(self
            .get_download_link(format!(
                "{album_id}/{}/cover.jpg",
                disc_id.map(|id| id.get()).unwrap_or(1)
            ))
            .await?))
    }
}

fn content_range_to_range(content_range: Option<&str>) -> Range {
    match content_range {
        Some(content_range) => {
            // if content range header is invalid, return the full range
            if content_range.len() <= 6 {
                return Range::FULL;
            }

            // else, parse the range
            // Content-Range: bytes 0-1023/10240
            //                      | offset = 6
            let content_range = &content_range[6..];
            let (from, content_range) =
                content_range.split_once('-').unwrap_or((content_range, ""));
            let (to, total) = content_range.split_once('/').unwrap_or((content_range, ""));

            Range {
                start: from.parse().unwrap_or(0),
                end: to.parse().ok(),
                total: total.parse().ok(),
            }
        }
        None => Range::FULL,
    }
}

fn to_io_error<T, E: Into<Box<dyn std::error::Error + Send + Sync>>>(
    r: Result<T, E>,
) -> Result<T, std::io::Error> {
    r.map_err(|e: E| std::io::Error::new(std::io::ErrorKind::Other, e))
}

async fn read_header<R>(mut reader: R) -> anni_provider::Result<(BlockStreamInfo, ResourceReader)>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let first = reader.read_u32().await.unwrap();
    let second = reader.read_u32().await.unwrap();
    let info = BlockStreamInfo::from_async_reader(&mut reader).await?;

    let mut header = Cursor::new(Vec::with_capacity(4 + 4 + 34));
    header.write_u32(first).await.unwrap();
    header.write_u32(second).await.unwrap();
    info.write_to(&mut header).unwrap();
    header.set_position(0);

    Ok((info, Box::pin(header.chain(reader))))
}

pub(crate) async fn read_duration(
    reader: ResourceReader,
    range: Range,
) -> anni_provider::Result<(u64, ResourceReader)> {
    if !range.contains_flac_header() {
        return Ok((0, reader));
    }

    let (info, reader) = read_header(reader).await?;
    let duration = info.total_samples / info.sample_rate as u64;
    Ok((duration, reader))
}

async fn read_response(resp: Response) -> anni_provider::Result<(u64, ResourceReader)> {
    let range = content_range_to_range(
        resp.headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok()),
    );
    let reader = StreamReader::new(resp.bytes_stream().map(to_io_error));
    read_duration(Box::pin(reader), range).await
}

fn handle_dav_error(e: reqwest_dav::Error) -> anni_provider::ProviderError {
    match e {
        reqwest_dav::Error::Reqwest(e) => e.into(),
        _ => anni_provider::ProviderError::GeneralError,
    }
}

pub trait AnniURLProvider: AnniProvider {
    fn get_audio_link(
        &self,
        album_id: &str,
        disc_id: NonZeroU8,
        track_id: NonZeroU8,
        range: Range,
    ) -> impl Future<Output = anni_provider::Result<Result<String, AudioResourceReader>>>
           + Send {
        async move {
            self.get_audio(album_id, disc_id, track_id, range)
                .await
                .map(Result::Err)
        }
    }

    fn get_cover_link(
        &self,
        album_id: &str,
        disc_id: Option<NonZeroU8>,
    ) -> impl Future<Output = anni_provider::Result<Result<String, ResourceReader>>> + Send
    {
        async move { self.get_cover(album_id, disc_id).await.map(Result::Err) }
    }
}
