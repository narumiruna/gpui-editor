use std::sync::{Arc, LazyLock};

use crate::{
    EditorState, Extension, Facet,
    autocomplete::session::{CompletionItem, SourceResult},
};

use super::{
    XmlAttrSpec, XmlAttribute, XmlDocument, XmlElementSpec, XmlSchema, XmlValue,
    xml_completion_source,
};

/// Register an in-memory schema for native XML completion. The first
/// registration in extension precedence order wins. No schema is read from the
/// document, file system, or network.
#[derive(Debug)]
pub struct NativeXmlSchemaExtension {
    schema: Arc<Schema>,
}

#[derive(Debug)]
struct Schema {
    elements: Vec<Element>,
    attributes: Vec<Attribute>,
}

type SchemaFacet = Facet<Arc<Schema>, Option<Arc<NativeXmlSchemaExtension>>>;
static SCHEMA: LazyLock<SchemaFacet> = LazyLock::new(|| {
    Facet::define(
        |inputs: &[&Arc<Schema>]| {
            inputs.first().map(|schema| {
                Arc::new(NativeXmlSchemaExtension {
                    schema: Arc::clone(schema),
                })
            })
        },
        |options| {
            options.compare(|next, old| match (next, old) {
                (Some(next), Some(old)) => Arc::ptr_eq(&next.schema, &old.schema),
                (None, None) => true,
                _ => false,
            })
        },
    )
});

impl NativeXmlSchemaExtension {
    /// Copy a schema so that the registration does not borrow host data.
    pub fn new(schema: XmlSchema<'_>) -> Self {
        Self {
            schema: Arc::new(Schema {
                elements: schema.elements.iter().map(Element::new).collect(),
                attributes: schema.attributes.iter().map(Attribute::new).collect(),
            }),
        }
    }

    pub fn extension(self) -> Extension {
        SCHEMA.of(self.schema)
    }

    /// Get the current source. Its `Arc` identity changes when the winning
    /// registration changes, even if a host later restores an earlier schema.
    pub fn source_for(state: &EditorState) -> Option<Arc<Self>> {
        state
            .context()
            .facet(&SCHEMA)
            .expect("typed XML schema facet")
            .as_ref()
            .clone()
    }

    /// Run the XML source with the registered schema in place of an empty one.
    pub fn complete(
        &self,
        doc: &XmlDocument,
        position: usize,
        explicit: bool,
    ) -> Option<SourceResult> {
        self.with_schema(|schema| xml_completion_source(doc, position, explicit, schema))
    }

    pub(crate) fn with_schema<R>(&self, use_schema: impl FnOnce(XmlSchema<'_>) -> R) -> R {
        let views: Vec<_> = self.schema.elements.iter().map(ElementView::new).collect();
        let local_attributes: Vec<_> = views.iter().map(ElementView::attributes).collect();
        let elements: Vec<_> = views
            .iter()
            .zip(&local_attributes)
            .map(|(view, attrs)| view.spec(attrs))
            .collect();
        let values: Vec<_> = self
            .schema
            .attributes
            .iter()
            .map(Attribute::values)
            .collect();
        let attributes: Vec<_> = self
            .schema
            .attributes
            .iter()
            .zip(&values)
            .map(|(attr, values)| attr.spec(values.as_deref()))
            .collect();
        use_schema(XmlSchema {
            elements: &elements,
            attributes: &attributes,
        })
    }
}

#[derive(Debug)]
struct Element {
    name: String,
    children: Option<Vec<String>>,
    text_content: Vec<String>,
    top: bool,
    attributes: Vec<LocalAttribute>,
    completion: Option<CompletionItem>,
}

impl Element {
    fn new(spec: &XmlElementSpec<'_>) -> Self {
        let strings = |slice: &[&str]| slice.iter().map(|&s| s.to_owned()).collect();
        Self {
            name: spec.name.into(),
            children: spec.children.map(strings),
            text_content: strings(spec.text_content),
            top: spec.top,
            attributes: spec
                .attributes
                .iter()
                .map(|attr| match attr {
                    XmlAttribute::Named(name) => LocalAttribute::Named((*name).into()),
                    XmlAttribute::Inline(spec) => {
                        LocalAttribute::Inline(Box::new(Attribute::new(spec)))
                    }
                })
                .collect(),
            completion: spec.completion.map(CompletionItem::from),
        }
    }
}

#[derive(Debug)]
enum LocalAttribute {
    Named(String),
    Inline(Box<Attribute>),
}

#[derive(Debug)]
struct Attribute {
    name: String,
    values: Option<Vec<Value>>,
    global: bool,
    completion: Option<CompletionItem>,
}

#[derive(Debug)]
enum Value {
    Text(String),
    Completion(Box<CompletionItem>),
}

impl Attribute {
    fn new(spec: &XmlAttrSpec<'_>) -> Self {
        Self {
            name: spec.name.into(),
            global: spec.global,
            values: spec.values.map(|values| {
                values
                    .iter()
                    .map(|value| match value {
                        XmlValue::Text(text) => Value::Text((*text).into()),
                        XmlValue::Completion(item) => Value::Completion(Box::new((*item).into())),
                    })
                    .collect()
            }),
            completion: spec.completion.map(CompletionItem::from),
        }
    }

    fn values(&self) -> Option<Vec<XmlValue<'_>>> {
        self.values.as_ref().map(|values| {
            values
                .iter()
                .map(|value| match value {
                    Value::Text(text) => XmlValue::Text(text),
                    Value::Completion(item) => XmlValue::Completion(item.as_completion()),
                })
                .collect()
        })
    }

    fn spec<'a>(&'a self, values: Option<&'a [XmlValue<'a>]>) -> XmlAttrSpec<'a> {
        XmlAttrSpec {
            name: &self.name,
            values,
            global: self.global,
            completion: self.completion.as_ref().map(CompletionItem::as_completion),
        }
    }
}

// The borrowed completion API uses slices, including for nested specs. Keep
// their backing arrays live only for the query; returned candidates own all data.
struct ElementView<'a> {
    element: &'a Element,
    children: Option<Vec<&'a str>>,
    text_content: Vec<&'a str>,
    values: Vec<Option<Vec<XmlValue<'a>>>>,
}

impl<'a> ElementView<'a> {
    fn new(element: &'a Element) -> Self {
        let strings = |list: &'a [String]| list.iter().map(String::as_str).collect();
        Self {
            element,
            children: element.children.as_deref().map(strings),
            text_content: strings(&element.text_content),
            values: element
                .attributes
                .iter()
                .map(|attr| match attr {
                    LocalAttribute::Named(_) => None,
                    LocalAttribute::Inline(attr) => attr.values(),
                })
                .collect(),
        }
    }

    fn attributes(&self) -> Vec<XmlAttribute<'_>> {
        self.element
            .attributes
            .iter()
            .zip(&self.values)
            .map(|(attr, values)| match attr {
                LocalAttribute::Named(name) => XmlAttribute::Named(name),
                LocalAttribute::Inline(attr) => XmlAttribute::Inline(attr.spec(values.as_deref())),
            })
            .collect()
    }

    fn spec<'s>(&'s self, attributes: &'s [XmlAttribute<'s>]) -> XmlElementSpec<'s> {
        XmlElementSpec {
            name: &self.element.name,
            children: self.children.as_deref(),
            text_content: &self.text_content,
            top: self.element.top,
            attributes,
            completion: self
                .element
                .completion
                .as_ref()
                .map(CompletionItem::as_completion),
        }
    }
}
