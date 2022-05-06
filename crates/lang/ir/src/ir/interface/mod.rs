// Copyright 2018-2022 Parity Technologies (UK) Ltd.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::{
    error::ExtError as _,
    ir,
    ir::utils,
};
use proc_macro2::{
    Ident,
    Span,
    TokenStream as TokenStream2,
};
use syn::{Result, spanned::Spanned as _};
use crate::ir::trait_def::TraitDefinitionConfig;

/// A checked ink! event definition.
#[derive(Debug, PartialEq, Eq)]
pub struct Interface {
    pub item: syn::ItemMod,
    pub trait_def: ir::InkTraitDefinition,
    pub event_def: Option<ir::EventDefinition>,
}

impl TryFrom<syn::ItemMod> for Interface {
    type Error = syn::Error;

    fn try_from(item: syn::ItemMod) -> Result<Self> {
        let (_, items) = item.content
            .ok_or_else(|| format_err!("#[ink::interface] must not be an empty module"))?;
        let item_trait = items.iter().find_map(|item|)


        let trait_def = ir::InkTraitDefinition::from_raw_parts(config, ink_item_trait);
        Ok(Self {
            item,
            trait_def,
            event_def,
        })
    }
}

impl quote::ToTokens for Interface {
    /// We mainly implement this trait for this ink! type to have a derived
    /// [`Spanned`](`syn::spanned::Spanned`) implementation for it.
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.item.to_tokens(tokens)
    }
}

impl Interface {
    /// Returns the identifier of the interface module.
    pub fn ident(&self) -> &Ident {
        &self.item.ident
    }

    /// Returns all non-ink! attributes.
    pub fn attrs(&self) -> &[syn::Attribute] {
        &self.item.attrs
    }
}