use std::{env, fs};
use crate::baserow::field_types::TableField;
use convert_case::{Case, Casing};
use quote::__private::TokenStream;
use quote::{format_ident, quote, TokenStreamExt};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client as ReqwestClient;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::io::read_to_string;

static LIST_TABLES_URL: &str = "https://api.baserow.io/api/database/tables/all-tables/";
static LIST_TABLE_FIELDS_URL: &str = "https://api.baserow.io/api/database/fields/table/";
static CREATE_RECORD_URL: &str = "https://api.baserow.io/api/database/rows/table/";
static LIST_RECORD_URL: &str = "https://api.baserow.io/api/database/rows/table/";

#[derive(Serialize, Deserialize, Debug)]
pub struct BaserowConfig {
    token: String,
    database: usize,
}

pub fn get_baserow_config() -> BaserowConfig {
    serde_json::from_str::<BaserowConfig>(&fs::read_to_string("baserow_config.json").unwrap()).unwrap()
}

pub fn get_database() -> String {
    env::var("BASEROW_DB").unwrap()
}

struct Client {
    client: ReqwestClient,
}

pub trait BaserowObject {
    fn get_static_table_id() -> usize;
    fn get_table_id(&self) -> usize;
    fn get_id(&self) -> Identifier;
    fn get_table_id_field(&self) -> String;
}

#[derive(Serialize, Deserialize, Debug)]
struct Table {
    pub id: usize,
    pub name: String,
    pub order: usize,
    pub database_id: usize,
    pub fields: Option<Vec<TableField>>,
}

pub enum Identifier {
    UnsignedNumber { id: Option<usize> },
    SignedNumber { id: Option<isize> },
    FloatNumber { id: Option<f64> },
    Text { id: Option<String> },
}

impl Identifier {
    pub fn get_string(&self) -> Option<String> {
        match self {
            Identifier::SignedNumber { id } => id.as_ref().map(|id| id.to_string()),
            Identifier::Text { id } => id.clone(),
            Identifier::UnsignedNumber { id } => id.as_ref().map(|id| id.to_string()),
            Identifier::FloatNumber { id } => id.as_ref().map(|id| id.to_string()),
        }
    }
}

impl Table {
    pub fn extend_with_fields(&mut self, fields: Vec<TableField>) {
        self.fields = Some(fields);
    }

    pub fn get_struct_name(&self) -> String {
        self.name.to_case(Case::Pascal)
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct SearchResult<T> {
    pub count: usize,
    pub next: Option<String>,
    pub previous: Option<String>,
    pub results: Vec<T>,
}

#[derive(Serialize, Deserialize, Debug)]
struct IdOnly {
    pub id: usize,
}

impl Client {
    pub fn new(token: &str) -> Self {
        let mut default_headers = HeaderMap::new();
        default_headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Token {}", token)).unwrap(),
        );
        default_headers.insert(ACCEPT, HeaderValue::from_str("application/json").unwrap());

        Self {
            client: ReqwestClient::builder()
                .default_headers(default_headers)
                .build()
                .unwrap(),
        }
    }

    async fn list_tables(&self) -> Vec<Table> {
        let mut tables = self
            .client
            .get(LIST_TABLES_URL)
            .send()
            .await
            .unwrap()
            .json::<Vec<Table>>()
            .await
            .unwrap();

        for table in &mut tables {
            if let Some(table_fields) = self.list_table_fields(&table.id).await {
                table.extend_with_fields(table_fields);
            }
        }

        tables
    }

    async fn list_table_fields(&self, table_id: &usize) -> Option<Vec<TableField>> {
        if let Ok(response) = self
            .client
            .get(format!("{LIST_TABLE_FIELDS_URL}{table_id}/"))
            .send()
            .await
        {
            Some(response.json::<Vec<TableField>>().await.unwrap())
        } else {
            None
        }
    }

    pub async fn list<T>(&self) -> Vec<T>
    where
        T: BaserowObject + Serialize + DeserializeOwned,
    {
        let list_url = format!("{CREATE_RECORD_URL}{}/", T::get_static_table_id());

        let response = self
            .client
            .get(list_url)
            .send()
            .await
            .unwrap()
            .json::<SearchResult<T>>()
            .await
            .unwrap();

        Vec::new()
    }

    pub async fn create<T>(&self, obj: &T)
    where
        T: BaserowObject + Serialize,
    {
        let create_url = format!("{CREATE_RECORD_URL}{}/", obj.get_table_id().to_string());

        let request = self
            .client
            .post(create_url)
            .header(CONTENT_TYPE, "application/json")
            .body(serde_json::to_string(obj).unwrap())
            .build()
            .unwrap();

        println!("Request\n{:?}", request);
        let response = self.client.execute(request).await.unwrap();

        println!("{:?}", response);
        println!("{:?}", response.text().await.unwrap())
    }

    pub async fn update<T>(&self, obj: &T)
    where
        T: BaserowObject + Serialize,
    {
        // Need to find the rowid for the object first
        let id: String = obj.get_id().get_string().unwrap();
        let find_url = format!(
            "{LIST_RECORD_URL}{}/?filter__{}__equal={}",
            obj.get_table_id(),
            obj.get_table_id_field(),
            id
        );

        println!("{:?}", self.client);
        let mut search_result = self
            .client
            .get(find_url)
            .send()
            .await
            .unwrap()
            .json::<SearchResult<IdOnly>>()
            .await
            .unwrap();

        if !search_result.count.eq(&1) {
            panic!("Should only have found one object for primary id!");
        }

        let id: String = search_result.results.first().unwrap().id.to_string();

        let update_url = format!(
            "{CREATE_RECORD_URL}{}/{}/",
            obj.get_table_id().to_string(),
            id
        );
        println!("{}", update_url);

        println!("{:?}", self.client);
        let response = self
            .client
            .patch(update_url)
            .header(CONTENT_TYPE, "application/json")
            .body(serde_json::to_string(obj).unwrap())
            .send()
            .await
            .unwrap();
        println!("{:?}", response);
        println!("{:?}", response.text().await.unwrap());
    }

    pub async fn generate_structs(&self, database: usize) {
        let tablelist = self.list_tables().await;

        let mut structs = quote! {
                    use crate::baserow::client::{BaserowObject, Identifier};
        use serde::de::Visitor;
        use serde::{de, Deserialize, Deserializer, Serialize};
        use std::fmt;
        use std::fmt::Write;
        use std::str::FromStr;
                };
        println!("{:?}", tablelist);
        for table in tablelist {
            // Gather information to be used during generation
            let struct_name = format_ident!("{}", table.get_struct_name());
            let fields = generate_fields(table.fields.as_ref());
            let primary_field = get_primary_field(table.fields.as_ref());
            let primary_field_id = format!("field_{}", primary_field.get_id());
            let primary_id_function = generate_primary_id_fn(primary_field);
            let table_id = table.id;

            // Generate code
            structs.extend(quote! {
                #[derive(Serialize, Deserialize, Debug, Clone)]
                pub struct #struct_name {
                    #fields
                }
                impl BaserowObject for #struct_name {
                    fn get_static_table_id() -> usize {
                        #table_id
                    }

                    fn get_table_id(&self) -> usize {
                        Self::get_static_table_id()
                    }

                    fn get_id(&self) -> Identifier {
                        #primary_id_function
                    }

                    fn get_table_id_field(&self) -> String {
                        #primary_field_id.to_string()
                    }
            }});
        }
        structs.extend(generate_deserializers());

        // Print formated code to stdout
        let syntax_tree = syn::parse_file(&structs.to_string()).unwrap();
        println!("{}", prettyplease::unparse(&syntax_tree));
    }
}

fn get_primary_field(fields: Option<&Vec<TableField>>) -> &TableField {
    if let Some(fields) = fields {
        let mut filtered_primary_fields = fields
            .iter()
            .filter(|field| field.is_primary())
            .collect::<Vec<&TableField>>();
        if filtered_primary_fields.len() != 1 {
            panic!("got more or less than one primary field!");
        }
        filtered_primary_fields.first().unwrap()
    } else {
        panic!("got no fields to determine primary field");
    }
}

fn generate_deserializers() -> TokenStream {
    quote! {
        fn isize_or_null<'de, D>(deserializer: D) -> Result<Option<isize>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct IsizeOrNull;

        impl<'de> Visitor<'de> for IsizeOrNull {
            type Value = Option<isize>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("number or null")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Some(isize::from_str(value).unwrap()))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(None)
            }
        }

        deserializer.deserialize_any(IsizeOrNull)
    }


    fn usize_or_null<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UsizeOrNull;

        impl<'de> Visitor<'de> for UsizeOrNull {
            type Value = Option<usize>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("number or null")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Some(usize::from_str(value).unwrap()))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(None)
            }
        }

        deserializer.deserialize_any(UsizeOrNull)
    }

    fn float_or_null<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FloatOrNull;

        impl<'de> Visitor<'de> for FloatOrNull {
            type Value = Option<f64>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("number or null")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(Some(f64::from_str(value).unwrap()))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(None)
            }
        }

        deserializer.deserialize_any(FloatOrNull)
    }
        }
}

fn generate_single_select_enum(field: &TableField) -> Option<TokenStream> {
    None
}

fn generate_fields(fields: Option<&Vec<TableField>>) -> Option<TokenStream> {
    if let Some(fields) = fields {
        let mut field_stream = TokenStream::new();
        for field in fields {
            /*match field {
                TableField::SingleSelect { shared_fields, select_options, single_select_default } => {
                    // Need to generate an enum to represent this field later on
                    #[derive(Serialize, Deserialize, Debug, Clone)]
                    #[serde(tag = "value")]
                    pub enum Status {
                        #[serde(rename = "draft")]
                        Draft {color: String, id: usize},
                        #[serde(rename = "sent")]
                        Sent {color: String, id: usize  },
                    }
                },
                _ => {}
            };*/
            let field_name = format_ident!("{}", field.get_name().to_case(Case::Snake));
            let field_type = format_ident!("{}", field.get_rust_type());
            let field_id = format!("field_{}", field.get_id());
            let deserializer = field.get_deserializer();

            field_stream.extend(quote! {
                #[serde(rename = #field_id #deserializer)]
                pub #field_name: Option<#field_type>,
            });
        }
        Some(field_stream)
    } else {
        None
    }
}

fn generate_primary_id_fn(primary_field: &TableField) -> TokenStream {
    let field_name = format_ident!("{}", primary_field.get_name());

    match primary_field.get_rust_type() {
        "isize" => quote! {
        Identifier::SignedNumber { id: self.#field_name}
        },
        "usize" => quote! {
        Identifier::UnsignedNumber { id: self.#field_name}
        },
        "f64" => quote! {
        Identifier::FloatNumber { id: self.#field_name}
        },
        _ => quote! {
        Identifier::Text { id: self.#field_name.clone()}
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::baserow::client::{get_baserow_config, Client, SearchResult};
    use std::fs;


    #[tokio::test]
    async fn generate_code() {
        let baserow_config = get_baserow_config();
        let client = Client::new(&baserow_config.token);
        client.generate_structs(baserow_config.database).await;
    }

    #[tokio::test]
    async fn test_list_offer() {
        let baserow_config = get_baserow_config();
        let client = Client::new(&baserow_config.token);
        let offers: Vec<Offers> = client.list().await;
        println!("{:?}", offers);
    }

    use crate::baserow::generated::Offers;
    #[test]
    fn test_deserialize_offerlist() {
        let contents = fs::read_to_string("testdata/list_offers1.json")
            .expect("Should have been able to read the file");

        let json: SearchResult<Offers> =
            serde_json::from_str(&contents).expect("file should be proper JSON");

        println!("{:?}", json)
    }



}
