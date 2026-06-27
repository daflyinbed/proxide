use crate::error::{ApiErrorDetail, WebError, WebResult};
use crate::repository::{PackageDownloadRow, UpstreamPackageDownloadRow};
use crate::state::AppState;
use axum::extract::Path;
use axum::Json;
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsPoint {
    pub downloads: u64,
    pub package: String,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsRange {
    pub package: String,
    pub start: String,
    pub end: String,
    pub downloads: Vec<DayDownloads>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DayDownloads {
    pub day: String,
    pub downloads: u64,
}

#[derive(Debug, Deserialize)]
pub struct NpmDownloadDay {
    pub day: String,
    pub downloads: u64,
}

fn sum_row_days(row: &PackageDownloadRow, start: NaiveDate, end: NaiveDate) -> u64 {
    let days: [u32; 31] = [
        row.d01, row.d02, row.d03, row.d04, row.d05, row.d06, row.d07, row.d08, row.d09, row.d10,
        row.d11, row.d12, row.d13, row.d14, row.d15, row.d16, row.d17, row.d18, row.d19, row.d20,
        row.d21, row.d22, row.d23, row.d24, row.d25, row.d26, row.d27, row.d28, row.d29, row.d30,
        row.d31,
    ];
    let mut total = 0u64;
    let row_start = NaiveDate::from_ymd_opt(row.year as i32, row.month as u32, 1)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
    let row_end = if row.month == 12 {
        NaiveDate::from_ymd_opt(row.year as i32 + 1, 1, 1).unwrap_or_else(|| {
            NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
        })
    } else {
        NaiveDate::from_ymd_opt(row.year as i32, row.month as u32 + 1, 1).unwrap_or_else(|| {
            NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
        })
    }
    .pred_opt()
    .unwrap_or(row_start);

    let eff_start = start.max(row_start);
    let eff_end = end.min(row_end);

    if eff_start > eff_end {
        return 0;
    }

    let mut d = eff_start;
    while d <= eff_end {
        let day_idx = (d.day() - 1) as usize;
        total += days[day_idx] as u64;
        d = d.succ_opt().unwrap_or(d);
    }
    total
}

fn sum_upstream_row_days(row: &UpstreamPackageDownloadRow, start: NaiveDate, end: NaiveDate) -> u64 {
    let days: [u32; 31] = [
        row.d01, row.d02, row.d03, row.d04, row.d05, row.d06, row.d07, row.d08, row.d09, row.d10,
        row.d11, row.d12, row.d13, row.d14, row.d15, row.d16, row.d17, row.d18, row.d19, row.d20,
        row.d21, row.d22, row.d23, row.d24, row.d25, row.d26, row.d27, row.d28, row.d29, row.d30,
        row.d31,
    ];
    let mut total = 0u64;
    let row_start = NaiveDate::from_ymd_opt(row.year as i32, row.month as u32, 1)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
    let row_end = if row.month == 12 {
        NaiveDate::from_ymd_opt(row.year as i32 + 1, 1, 1).unwrap_or_else(|| {
            NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
        })
    } else {
        NaiveDate::from_ymd_opt(row.year as i32, row.month as u32 + 1, 1).unwrap_or_else(|| {
            NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
        })
    }
    .pred_opt()
    .unwrap_or(row_start);

    let eff_start = start.max(row_start);
    let eff_end = end.min(row_end);

    if eff_start > eff_end {
        return 0;
    }

    let mut d = eff_start;
    while d <= eff_end {
        let day_idx = (d.day() - 1) as usize;
        total += days[day_idx] as u64;
        d = d.succ_opt().unwrap_or(d);
    }
    total
}

fn parse_range(range: &str) -> WebResult<(NaiveDate, NaiveDate)> {
    let today = chrono::Utc::now().date_naive();

    match range {
        "last-day" => {
            let start = today.pred_opt().unwrap_or(today);
            Ok((start, start))
        }
        "last-week" => {
            let end = today.pred_opt().unwrap_or(today);
            let start = (0..6).fold(end, |d, _| d.pred_opt().unwrap_or(d));
            Ok((start, end))
        }
        "last-month" => {
            let end = today.pred_opt().unwrap_or(today);
            let start = (0..29).fold(end, |d, _| d.pred_opt().unwrap_or(d));
            Ok((start, end))
        }
        "last-year" => {
            let end = today.pred_opt().unwrap_or(today);
            let start = (0..364).fold(end, |d, _| d.pred_opt().unwrap_or(d));
            Ok((start, end))
        }
        s if s.contains(':') => {
            let parts: Vec<&str> = s.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(WebError::BadRequest(format!("invalid date range: {s}")));
            }
            let start = NaiveDate::parse_from_str(parts[0], "%Y-%m-%d")
                .map_err(|_| WebError::BadRequest(format!("invalid start date: {}", parts[0])))?;
            let end = NaiveDate::parse_from_str(parts[1], "%Y-%m-%d")
                .map_err(|_| WebError::BadRequest(format!("invalid end date: {}", parts[1])))?;
            if start > end {
                return Err(WebError::BadRequest(format!("invalid date range: {s}")));
            }
            Ok((start, end))
        }
        s => {
            let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|_| WebError::BadRequest(format!("invalid date: {s}")))?;
            Ok((d, d))
        }
    }
}

fn parse_download_target(rest: &str) -> WebResult<(String, String)> {
    let rest = rest.trim_start_matches('/');
    let Some((range, fullname)) = rest.split_once('/') else {
        return Err(WebError::NotFound(format!("not found: {rest}")));
    };

    if fullname.is_empty() {
        return Err(WebError::NotFound(format!("not found: {rest}")));
    }

    Ok((range.to_string(), fullname.to_string()))
}

fn expand_to_day_map(
    rows: &[UpstreamPackageDownloadRow],
    start: NaiveDate,
    end: NaiveDate,
) -> BTreeMap<NaiveDate, u64> {
    let mut map = BTreeMap::new();
    for row in rows {
        let row_start = NaiveDate::from_ymd_opt(row.year as i32, row.month as u32, 1)
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
        let row_end = if row.month == 12 {
            NaiveDate::from_ymd_opt(row.year as i32 + 1, 1, 1).unwrap_or_else(|| {
                NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
            })
        } else {
            NaiveDate::from_ymd_opt(row.year as i32, row.month as u32 + 1, 1).unwrap_or_else(|| {
                NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
            })
        }
        .pred_opt()
        .unwrap_or(row_start);

        let days: [u32; 31] = [
            row.d01, row.d02, row.d03, row.d04, row.d05, row.d06, row.d07, row.d08, row.d09,
            row.d10, row.d11, row.d12, row.d13, row.d14, row.d15, row.d16, row.d17, row.d18,
            row.d19, row.d20, row.d21, row.d22, row.d23, row.d24, row.d25, row.d26, row.d27,
            row.d28, row.d29, row.d30, row.d31,
        ];

        let mut d = start.max(row_start);
        let boundary = end.min(row_end);
        while d <= boundary {
            let day_idx = (d.day() - 1) as usize;
            *map.entry(d).or_insert(0) += days[day_idx] as u64;
            d = d.succ_opt().unwrap_or(d);
        }
    }
    map
}

async fn fetch_upstream_range(
    state: &AppState,
    fullname: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> WebResult<Vec<NpmDownloadDay>> {
    let url = format!(
        "{}/downloads/range/{}:{}/{}",
        state.config.worker.upstream_registry,
        start.format("%Y-%m-%d"),
        end.format("%Y-%m-%d"),
        fullname
    );

    let mut request = state.http.get(&url);
    if !state.config.worker.upstream_auth_token.is_empty() {
        request = request.bearer_auth(&state.config.worker.upstream_auth_token);
    }

    let resp = request.send().await.map_err(|e| {
        WebError::CustomApiError(anyhow::anyhow!("failed to fetch upstream downloads: {e}"))
    })?;

    if !resp.status().is_success() {
        return Ok(vec![]);
    }

    #[derive(Deserialize)]
    struct UpstreamResponse {
        downloads: Vec<NpmDownloadDay>,
    }

    let body: UpstreamResponse = resp.json().await.map_err(|e| {
        WebError::CustomApiError(anyhow::anyhow!("failed to parse upstream download response: {e}"))
    })?;

    Ok(body.downloads)
}

async fn ensure_upstream_cache(
    state: &AppState,
    package_id: i64,
    fullname: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> WebResult<()> {
    let today = chrono::Utc::now().date_naive();

    let cached_end = today.pred_opt().unwrap_or(today);
    if start > cached_end {
        return Ok(());
    }

    let eff_end = end.min(cached_end);

    let upstream_data = fetch_upstream_range(state, fullname, start, eff_end).await?;

    for entry in &upstream_data {
        let day = match NaiveDate::parse_from_str(&entry.day, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => continue,
        };
        if day >= today {
            continue;
        }
        if let Err(e) = state
            .repo
            .upsert_upstream_download(
                package_id,
                day.year() as u16,
                day.month() as u8,
                day.day() as u8,
                entry.downloads,
            )
            .await
        {
            log::error!("failed to write upstream download cache: {e:#}");
        }
    }

    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/downloads/point/{rest}",
    tag = "downloads",
    params(
        ("rest" = String, Path, description = "`{range}/{fullname}` — e.g. `last-week/lodash`, `2024-01-01:2024-06-30/@babel/core`"),
    ),
    responses(
        (status = OK, body = DownloadsPoint, description = "Total download count for the range"),
        (status = BAD_REQUEST, body = ApiErrorDetail, description = "Invalid range"),
        (status = NOT_FOUND, body = ApiErrorDetail, description = "Package not found"),
    )
)]
pub async fn downloads_point(
    axum::extract::State(state): axum::extract::State<AppState>,
    Path(rest): Path<String>,
) -> WebResult<Json<DownloadsPoint>> {
    let (range, fullname) = parse_download_target(&rest)?;
    let (start, end) = parse_range(&range)?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let local_rows = state
        .repo
        .query_package_downloads_by_package(
            pkg.id,
            start.year() as u16,
            start.month() as u8,
            end.year() as u16,
            end.month() as u8,
        )
        .await
        .map_err(WebError::CustomApiError)?;

    let mut total: u64 = local_rows
        .iter()
        .map(|(_, _, row)| sum_row_days(row, start, end))
        .sum();

    let _ = ensure_upstream_cache(&state, pkg.id, &fullname, start, end).await;

    let today = chrono::Utc::now().date_naive();
    let cached_end = today.pred_opt().unwrap_or(today);

    if start <= cached_end {
        let eff_end = end.min(cached_end);
        let upstream_rows = state
            .repo
            .query_upstream_downloads(
                pkg.id,
                start.year() as u16,
                start.month() as u8,
                eff_end.year() as u16,
                eff_end.month() as u8,
            )
            .await
            .map_err(WebError::CustomApiError)?;
        total += upstream_rows
            .iter()
            .map(|r| sum_upstream_row_days(r, start, eff_end))
            .sum::<u64>();
    }

    if end >= today {
        let today_start = start.max(today);
        let upstream_data = fetch_upstream_range(&state, &fullname, today_start, end).await?;
        total += upstream_data.iter().map(|d| d.downloads).sum::<u64>();
    }

    Ok(Json(DownloadsPoint {
        downloads: total,
        package: fullname,
        start: start.format("%Y-%m-%d").to_string(),
        end: end.format("%Y-%m-%d").to_string(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/downloads/range/{rest}",
    tag = "downloads",
    params(
        ("rest" = String, Path, description = "`{range}/{fullname}` — e.g. `last-month/lodash`, `2024-01-01:2024-06-30/@babel/core`"),
    ),
    responses(
        (status = OK, body = DownloadsRange, description = "Per-day download counts for the range"),
        (status = BAD_REQUEST, body = ApiErrorDetail, description = "Invalid range"),
        (status = NOT_FOUND, body = ApiErrorDetail, description = "Package not found"),
    )
)]
pub async fn downloads_range(
    axum::extract::State(state): axum::extract::State<AppState>,
    Path(rest): Path<String>,
) -> WebResult<Json<DownloadsRange>> {
    let (range, fullname) = parse_download_target(&rest)?;
    let (start, end) = parse_range(&range)?;

    let pkg = state
        .repo
        .get_package_by_name(&fullname)
        .await
        .map_err(WebError::CustomApiError)?
        .ok_or_else(|| WebError::NotFound(format!("{fullname} not found")))?;

    let local_rows = state
        .repo
        .query_package_downloads_by_package(
            pkg.id,
            start.year() as u16,
            start.month() as u8,
            end.year() as u16,
            end.month() as u8,
        )
        .await
        .map_err(WebError::CustomApiError)?;

    let mut day_map: BTreeMap<NaiveDate, u64> = BTreeMap::new();

    for (_, _, row) in &local_rows {
        let row_start = NaiveDate::from_ymd_opt(row.year as i32, row.month as u32, 1)
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(2000, 1, 1).unwrap());
        let row_end = if row.month == 12 {
            NaiveDate::from_ymd_opt(row.year as i32 + 1, 1, 1).unwrap_or_else(|| {
                NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
            })
        } else {
            NaiveDate::from_ymd_opt(row.year as i32, row.month as u32 + 1, 1).unwrap_or_else(|| {
                NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()
            })
        }
        .pred_opt()
        .unwrap_or(row_start);

        let days: [u32; 31] = [
            row.d01, row.d02, row.d03, row.d04, row.d05, row.d06, row.d07, row.d08, row.d09,
            row.d10, row.d11, row.d12, row.d13, row.d14, row.d15, row.d16, row.d17, row.d18,
            row.d19, row.d20, row.d21, row.d22, row.d23, row.d24, row.d25, row.d26, row.d27,
            row.d28, row.d29, row.d30, row.d31,
        ];

        let mut d = start.max(row_start);
        let boundary = end.min(row_end);
        while d <= boundary {
            let day_idx = (d.day() - 1) as usize;
            *day_map.entry(d).or_insert(0) += days[day_idx] as u64;
            d = d.succ_opt().unwrap_or(d);
        }
    }

    let _ = ensure_upstream_cache(&state, pkg.id, &fullname, start, end).await;

    let today = chrono::Utc::now().date_naive();
    let cached_end = today.pred_opt().unwrap_or(today);

    if start <= cached_end {
        let eff_end = end.min(cached_end);
        let upstream_rows = state
            .repo
            .query_upstream_downloads(
                pkg.id,
                start.year() as u16,
                start.month() as u8,
                eff_end.year() as u16,
                eff_end.month() as u8,
            )
            .await
            .map_err(WebError::CustomApiError)?;
        let upstream_map = expand_to_day_map(&upstream_rows, start, eff_end);
        for (day, count) in upstream_map {
            *day_map.entry(day).or_insert(0) += count;
        }
    }

    if end >= today {
        let today_start = start.max(today);
        let upstream_data = fetch_upstream_range(&state, &fullname, today_start, end).await?;
        for entry in &upstream_data {
            let day = match NaiveDate::parse_from_str(&entry.day, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => continue,
            };
            *day_map.entry(day).or_insert(0) += entry.downloads;
        }
    }

    let mut result_days = Vec::new();
    let mut d = start;
    while d <= end {
        result_days.push(DayDownloads {
            day: d.format("%Y-%m-%d").to_string(),
            downloads: *day_map.get(&d).unwrap_or(&0),
        });
        d = d.succ_opt().unwrap_or(d);
    }

    Ok(Json(DownloadsRange {
        package: fullname,
        start: start.format("%Y-%m-%d").to_string(),
        end: end.format("%Y-%m-%d").to_string(),
        downloads: result_days,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_download_target_simple_package() {
        let (range, fullname) = parse_download_target("last-week/lodash").unwrap();
        assert_eq!(range, "last-week");
        assert_eq!(fullname, "lodash");
    }

    #[test]
    fn parse_download_target_scoped_package() {
        let (range, fullname) = parse_download_target("last-week/@babel/core").unwrap();
        assert_eq!(range, "last-week");
        assert_eq!(fullname, "@babel/core");
    }

    #[test]
    fn parse_range_last_week_is_seven_days_inclusive() {
        let today = chrono::Utc::now().date_naive();
        let (start, end) = parse_range("last-week").unwrap();

        assert_eq!(end, today.pred_opt().unwrap_or(today));
        assert_eq!(end.signed_duration_since(start).num_days(), 6);
    }

    #[test]
    fn parse_range_rejects_reversed_explicit_range() {
        let err = parse_range("2026-05-10:2026-05-01").unwrap_err();
        assert!(matches!(err, WebError::BadRequest(_)));
    }
}
