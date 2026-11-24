use super::*;

/// Lifecycle status of a customer account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CustomerStatus)]
pub enum CustomerStatusDto {
    /// The customer may authenticate and place orders.
    Active,
    /// The customer is disabled and cannot authenticate.
    Disabled,
}

impl From<CustomerStatus> for CustomerStatusDto {
    fn from(status: CustomerStatus) -> Self {
        match status {
            CustomerStatus::Active => Self::Active,
            CustomerStatus::Disabled => Self::Disabled,
        }
    }
}

/// A store-scoped customer profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = Customer)]
pub struct CustomerDto {
    /// Customer identifier.
    pub id: String,
    /// Normalized email address.
    pub email: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Contact phone number.
    pub phone: Option<String>,
    /// Whether the profile has registered credentials.
    #[schema(required = false)]
    pub has_account: bool,
    /// Lifecycle status.
    pub status: CustomerStatusDto,
    /// Email verification timestamp, absent until verified.
    pub email_verified_at: Option<i64>,
    /// Free-form extension metadata.
    #[schema(required = false)]
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    #[schema(required = false)]
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    #[schema(required = false)]
    pub updated_at: i64,
    /// Soft-deletion timestamp, present when retained for history.
    pub deleted_at: Option<i64>,
}

impl From<Customer> for CustomerDto {
    fn from(customer: Customer) -> Self {
        Self {
            id: customer.id.to_string(),
            email: customer.email,
            first_name: customer.first_name,
            last_name: customer.last_name,
            phone: customer.phone,
            has_account: customer.has_account,
            status: customer.status.into(),
            email_verified_at: customer.email_verified_at,
            metadata: customer.metadata,
            created_at: customer.created_at,
            updated_at: customer.updated_at,
            deleted_at: customer.deleted_at,
        }
    }
}

/// A customer segment used for pricing and promotions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = CustomerGroup)]
pub struct CustomerGroupDto {
    /// Group identifier.
    pub id: String,
    /// Unique human-readable name.
    pub name: String,
    /// Free-form extension metadata.
    #[schema(required = false)]
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    #[schema(required = false)]
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    #[schema(required = false)]
    pub updated_at: i64,
}

impl From<CustomerGroup> for CustomerGroupDto {
    fn from(group: CustomerGroup) -> Self {
        Self {
            id: group.id,
            name: group.name,
            metadata: group.metadata,
            created_at: group.created_at,
            updated_at: group.updated_at,
        }
    }
}

/// A customer postal address.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(as = CustomerAddress)]
pub struct CustomerAddressDto {
    /// Address identifier.
    pub id: String,
    /// Owning customer.
    pub customer_id: String,
    /// Whether this is the default shipping address.
    pub is_default_shipping: bool,
    /// Whether this is the default billing address.
    pub is_default_billing: bool,
    /// Recipient given name.
    pub first_name: Option<String>,
    /// Recipient family name.
    pub last_name: Option<String>,
    /// Company name.
    pub company: Option<String>,
    /// First address line.
    pub address_1: Option<String>,
    /// Second address line.
    pub address_2: Option<String>,
    /// City or locality.
    pub city: Option<String>,
    /// Province, region or state.
    pub province: Option<String>,
    /// Postal or ZIP code.
    pub postal_code: Option<String>,
    /// Two-letter country code.
    pub country_code: Option<String>,
    /// Contact phone number.
    pub phone: Option<String>,
    /// Free-form extension metadata.
    pub metadata: Metadata,
    /// Creation timestamp in Unix seconds.
    pub created_at: i64,
    /// Last update timestamp in Unix seconds.
    pub updated_at: i64,
}

impl From<Address> for CustomerAddressDto {
    fn from(address: Address) -> Self {
        Self {
            id: address.id,
            customer_id: address.customer_id.to_string(),
            is_default_shipping: address.is_default_shipping,
            is_default_billing: address.is_default_billing,
            first_name: address.first_name,
            last_name: address.last_name,
            company: address.company,
            address_1: address.address_1,
            address_2: address.address_2,
            city: address.city,
            province: address.province,
            postal_code: address.postal_code,
            country_code: address.country_code.map(|code| code.as_str().to_owned()),
            phone: address.phone,
            metadata: address.metadata,
            created_at: address.created_at,
            updated_at: address.updated_at,
        }
    }
}

/// A single customer address.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CustomerAddressResponse {
    /// The address.
    pub address: CustomerAddressDto,
}

/// All addresses of the authenticated customer.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CustomerAddressListResponse {
    /// Customer addresses.
    pub addresses: Vec<CustomerAddressDto>,
}

/// The groups the authenticated customer belongs to.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CustomerGroupsResponse {
    /// Customer groups.
    pub customer_groups: Vec<CustomerGroupDto>,
}

/// The authenticated storefront customer.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct CustomerResponse {
    /// Customer profile.
    pub customer: CustomerDto,
}
