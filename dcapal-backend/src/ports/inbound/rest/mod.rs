//! The [`rest`](self) module implements the REST API of the system

use std::{fmt::Display, time::Duration};

use axum::{
    Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use axum_extra::{TypedHeader, headers::CacheControl};
use hyper::StatusCode;
use lazy_static::lazy_static;
use metrics::counter;
use sea_orm::prelude::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    AppContext,
    app::{
        domain::entity::AssetKind,
        infra::utils::Expiring,
        services::command::{ConversionRateQuery, ImportPortfolioCmd},
    },
    error::{DcaError, Result},
    infra::stats,
    ports::outbound::repository::ImportedPortfolio,
};

pub mod request;
pub mod response;

static PORTFOLIO_SCHEMA_STR: &str =
    include_str!("../../../../docs/schema/portfolio/v1/schema.json");

lazy_static! {
    static ref ASSETS_CACHE_CONTROL: CacheControl = CacheControl::new()
        .with_public()
        .with_max_age(Duration::from_secs(5 * 60));
    static ref PORTFOLIO_JSON_SCHEMA: serde_json::Value =
        serde_json::from_str(PORTFOLIO_SCHEMA_STR).unwrap();
    static ref PORTFOLIO_SCHEMA_VALIDATOR: jsonschema::Validator =
        jsonschema::draft7::new(&PORTFOLIO_JSON_SCHEMA).unwrap();
}

pub async fn get_assets_fiat(State(ctx): State<AppContext>) -> Result<Response> {
    let service = &ctx.services.mkt_data;

    let assets = service.get_assets_by_type(AssetKind::Fiat).await;

    let response = (
        TypedHeader(ASSETS_CACHE_CONTROL.clone()),
        Json((*assets).clone()),
    );

    Ok(response.into_response())
}

pub async fn get_assets_crypto(State(ctx): State<AppContext>) -> Result<Response> {
    let service = &ctx.services.mkt_data;

    let assets = service.get_assets_by_type(AssetKind::Crypto).await;

    let response = (
        TypedHeader(ASSETS_CACHE_CONTROL.clone()),
        Json((*assets).clone()),
    );

    Ok(response.into_response())
}

#[derive(Debug, Deserialize)]
pub struct GetAssetsQuery {
    name: String,
}

pub async fn get_assets_data(
    State(ctx): State<AppContext>,
    Query(params): Query<GetAssetsQuery>,
) -> Result<Response> {
    Ok(ctx.providers.yahoo.search(params.name).await)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAssetChartQuery {
    start_period: i64,
    end_period: i64,
}

pub async fn get_assets_chart(
    Path(asset): Path<String>,
    Query(params): Query<GetAssetChartQuery>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    Ok(ctx
        .providers
        .yahoo
        .chart(asset, params.start_period, params.end_period)
        .await)
}

#[derive(Debug, Deserialize)]
pub struct GetPriceQuery {
    quote: String,
}

pub async fn get_price(
    Path(asset): Path<String>,
    Query(query): Query<GetPriceQuery>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let repo = &ctx.repos.mkt_data;
    let service = &ctx.services.mkt_data;

    let cmd = ConversionRateQuery::try_new(&asset, &query.quote, repo).await?;
    let (base, quote) = (cmd.base.id().clone(), cmd.quote.id().clone());

    let price = service
        .get_conversion_rate(cmd)
        .await?
        .ok_or(DcaError::PriceNotAvailable(base, quote))?;

    let response = (TypedHeader(cache_control(&price)), Json(price));
    Ok(response.into_response())
}

fn cache_control<T: Expiring>(t: &T) -> CacheControl {
    CacheControl::new()
        .with_public()
        .with_max_age(Duration::from_secs(t.time_to_live().as_secs()))
}

#[derive(Debug, Serialize)]
pub struct ImportPortfolioResponse {
    pub id: String,
    pub expires_at: String,
}

impl From<ImportedPortfolio> for ImportPortfolioResponse {
    fn from(value: ImportedPortfolio) -> Self {
        Self {
            id: value.id.simple().to_string(),
            expires_at: value.expires_at.to_string(),
        }
    }
}

pub async fn import_portfolio(
    State(ctx): State<AppContext>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Response> {
    let repo = &ctx.repos.imported;
    let stats_repo = &ctx.repos.stats;

    let cmd = ImportPortfolioCmd::try_new(payload, &PORTFOLIO_SCHEMA_VALIDATOR)?;
    let imported = repo.store_portfolio(&cmd.pfolio).await?;

    counter!(stats::IMPORTED_PORTFOLIOS_TOTAL).increment(1);
    let _ = stats_repo.increase_imported_portfolio_count().await;

    let response = (
        StatusCode::CREATED,
        Json(ImportPortfolioResponse::from(imported)),
    );

    Ok(response.into_response())
}

pub async fn get_imported_portfolio(
    Path(id): Path<String>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let repo = &ctx.repos.imported;

    match repo.find_portfolio(&id).await? {
        Some(portfolio) => Ok(Json(portfolio).into_response()),
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FeeStructure {
    #[serde(rename = "zeroFee")]
    ZeroFee,

    #[serde(rename = "fixed")]
    Fixed {
        #[serde(rename = "feeAmount", with = "rust_decimal::serde::float")]
        fee_amount: Decimal,
    },

    #[serde(rename = "variable")]
    Variable {
        #[serde(rename = "feeRate", with = "rust_decimal::serde::float")]
        fee_rate: Decimal,
        #[serde(rename = "minFee", with = "rust_decimal::serde::float")]
        min_fee: Decimal,
        #[serde(
            rename = "maxFee",
            default,
            skip_serializing_if = "Option::is_none",
            with = "rust_decimal::serde::float_option"
        )]
        max_fee: Option<Decimal>,
    },
}

impl Display for FeeStructure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeeStructure::ZeroFee => write!(f, "ZeroFee"),
            FeeStructure::Fixed { .. } => write!(f, "Fixed"),
            FeeStructure::Variable { .. } => write!(f, "Variable"),
        }
    }
}
