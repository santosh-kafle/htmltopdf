//! Mapping CSS font families to parley font stacks.

use std::borrow::Cow;

use parley::{FontContext, FontFamily as FamilyStack, FontFamilyName as StackName};
use presswork_style::{FontFamilyName, GenericFamily};

/// The font each generic family resolves to.
///
/// fontconfig maps a generic family such as `serif` to a long list of fonts
/// covering many scripts, in an order that is not script-aware: Noto Serif
/// Bengali can come before Noto Serif Devanagari, and then wins characters
/// both contain, such as the danda (।). Browsers instead map each generic
/// family to one font and pick fallback fonts by script, so do the same.
#[derive(Debug, Default)]
pub(crate) struct GenericFonts {
    serif: Option<String>,
    sans_serif: Option<String>,
    monospace: Option<String>,
}

impl GenericFonts {
    pub fn resolve(fonts: &mut FontContext) -> Self {
        let mut first = |generic| {
            let id = fonts.collection.generic_families(generic).next()?;
            fonts.collection.family_name(id).map(str::to_owned)
        };
        Self {
            serif: first(parley::GenericFamily::Serif),
            sans_serif: first(parley::GenericFamily::SansSerif),
            monospace: first(parley::GenericFamily::Monospace),
        }
    }

    /// The parley font stack for a computed `font-family` list.
    pub fn stack<'a>(&'a self, families: &'a [FontFamilyName]) -> FamilyStack<'a> {
        let names = families
            .iter()
            .map(|family| match family {
                FontFamilyName::Named(name) => StackName::named(name),
                FontFamilyName::Generic(generic) => self.generic(*generic),
            })
            .collect();
        FamilyStack::List(Cow::Owned(names))
    }

    fn generic(&self, generic: GenericFamily) -> StackName<'_> {
        let (name, fallback) = match generic {
            GenericFamily::Serif => (&self.serif, parley::GenericFamily::Serif),
            GenericFamily::SansSerif => (&self.sans_serif, parley::GenericFamily::SansSerif),
            GenericFamily::Monospace => (&self.monospace, parley::GenericFamily::Monospace),
        };
        match name {
            Some(name) => StackName::named(name),
            None => fallback.into(),
        }
    }
}
