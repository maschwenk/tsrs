use tsrs_lsproto as lsproto;

use crate::lsutil::{ScriptElementKind, ScriptElementKindModifier};

// vsImageCatalogGuid is the GUID of the shared VS image catalog (see
// Microsoft.VisualStudio.Imaging.KnownImageIds), mirroring the constant duplicated in
// TypeScript-VS's ImageIdMapping.cs (which avoids taking an assembly reference just for this GUID).
// hovericon.go:11
const VS_IMAGE_CATALOG_GUID: &str = "ae27a6b0-e345-4288-96df-5eaf394ee369";

// Known image IDs from Microsoft.VisualStudio.Imaging.KnownImageIds, restricted to the subset
// consumed by TypeScript-VS's ImageIdMapping.cs for hover tooltips. Corsa (this LSP server) has no
// TSServer/Roslyn dependency to reuse that mapping from, so the values are duplicated here.
// hovericon.go:16
const IMAGE_ID_WARNING: i32 = 0x00000637;
const IMAGE_ID_KEYWORD: i32 = 0x00000635;
const IMAGE_ID_MODULE_PRIVATE: i32 = 0x0000077D;
const IMAGE_ID_MODULE_PROTECTED: i32 = 0x0000077E;
const IMAGE_ID_MODULE_PUBLIC: i32 = 0x0000077F;
const IMAGE_ID_TYPE: i32 = 0x00000CA1;
const IMAGE_ID_NAMESPACE: i32 = 0x0000079F;
const IMAGE_ID_CLASS_PRIVATE: i32 = 0x000001D7;
const IMAGE_ID_CLASS_PROTECTED: i32 = 0x000001D8;
const IMAGE_ID_CLASS_PUBLIC: i32 = 0x000001D9;
const IMAGE_ID_INTERFACE_PRIVATE: i32 = 0x00000646;
const IMAGE_ID_INTERFACE_PROTECTED: i32 = 0x00000647;
const IMAGE_ID_INTERFACE_PUBLIC: i32 = 0x00000648;
const IMAGE_ID_ENUM_PRIVATE: i32 = 0x00000469;
const IMAGE_ID_ENUM_PROTECTED: i32 = 0x0000046A;
const IMAGE_ID_ENUM_PUBLIC: i32 = 0x0000046B;
const IMAGE_ID_ENUM_MEMBER: i32 = 0x00000465;
const IMAGE_ID_LOCAL_VARIABLE: i32 = 0x000006D3;
const IMAGE_ID_PROPERTY_PRIVATE: i32 = 0x00000982;
const IMAGE_ID_PROPERTY_PROTECTED: i32 = 0x00000983;
const IMAGE_ID_PROPERTY_PUBLIC: i32 = 0x00000984;
const IMAGE_ID_METHOD_PRIVATE: i32 = 0x00000756;
const IMAGE_ID_METHOD_PROTECTED: i32 = 0x00000757;
const IMAGE_ID_METHOD_PUBLIC: i32 = 0x00000758;
const IMAGE_ID_LABEL: i32 = 0x0000067D;
const IMAGE_ID_ASSEMBLY: i32 = 0x000000C4;
const IMAGE_ID_CONSTANT_PRIVATE: i32 = 0x0000026A;
const IMAGE_ID_CONSTANT_PROTECTED: i32 = 0x0000026B;
const IMAGE_ID_CONSTANT_PUBLIC: i32 = 0x0000026C;

// hovericon.go:48
fn new_vs_image_id(id: i32) -> lsproto::VSImageId {
    lsproto::VSImageId { guid: VS_IMAGE_CATALOG_GUID.to_string(), id, ..Default::default() }
}

// getVSHoverImageId maps a symbol's ScriptElementKind/modifiers to the VS image shown next to the
// symbol name in hover tooltips. This mirrors TypeScript-VS's ImageIdMapping.GetImageId, which the
// legacy (TSServer-backed) hover path uses; Corsa has no TSServer to source that mapping from, so
// the LSP hover response must carry the equivalent icon directly.
// hovericon.go:56
pub(crate) fn get_vs_hover_image_id(kind: ScriptElementKind, modifiers: ScriptElementKindModifier) -> lsproto::VSImageId {
    let is_private = modifiers.intersects(ScriptElementKindModifier::Private);
    let is_protected = modifiers.intersects(ScriptElementKindModifier::Protected);

    // No internal/exported arm: the *Internal VS icons carry a chevron overlay that conveys
    // C# assembly-scoped visibility, a concept that doesn't apply to TypeScript.
    let pick = |private: i32, protected: i32, public: i32| -> lsproto::VSImageId {
        if is_private {
            new_vs_image_id(private)
        } else if is_protected {
            new_vs_image_id(protected)
        } else {
            new_vs_image_id(public)
        }
    };

    use ScriptElementKind as K;
    match kind {
        K::Warning => new_vs_image_id(IMAGE_ID_WARNING),
        K::Keyword => new_vs_image_id(IMAGE_ID_KEYWORD),
        K::ScriptElement => pick(IMAGE_ID_MODULE_PRIVATE, IMAGE_ID_MODULE_PROTECTED, IMAGE_ID_MODULE_PUBLIC),
        K::PrimitiveType => new_vs_image_id(IMAGE_ID_TYPE),
        K::ModuleElement => new_vs_image_id(IMAGE_ID_NAMESPACE),
        K::ConstructorImplementationElement | K::ClassElement | K::LocalClassElement | K::TypeElement => {
            pick(IMAGE_ID_CLASS_PRIVATE, IMAGE_ID_CLASS_PROTECTED, IMAGE_ID_CLASS_PUBLIC)
        }
        K::InterfaceElement => pick(IMAGE_ID_INTERFACE_PRIVATE, IMAGE_ID_INTERFACE_PROTECTED, IMAGE_ID_INTERFACE_PUBLIC),
        K::EnumElement => pick(IMAGE_ID_ENUM_PRIVATE, IMAGE_ID_ENUM_PROTECTED, IMAGE_ID_ENUM_PUBLIC),
        K::EnumMemberElement => new_vs_image_id(IMAGE_ID_ENUM_MEMBER),
        K::ParameterElement
        | K::VariableElement
        | K::LocalVariableElement
        | K::VariableUsingElement
        | K::VariableAwaitUsingElement
        | K::LetElement
        | K::String => new_vs_image_id(IMAGE_ID_LOCAL_VARIABLE),
        K::ConstElement => pick(IMAGE_ID_CONSTANT_PRIVATE, IMAGE_ID_CONSTANT_PROTECTED, IMAGE_ID_CONSTANT_PUBLIC),
        K::MemberGetAccessorElement | K::MemberSetAccessorElement | K::MemberVariableElement | K::MemberAccessorVariableElement => {
            pick(IMAGE_ID_PROPERTY_PRIVATE, IMAGE_ID_PROPERTY_PROTECTED, IMAGE_ID_PROPERTY_PUBLIC)
        }
        K::FunctionElement
        | K::LocalFunctionElement
        | K::MemberFunctionElement
        | K::CallSignatureElement
        | K::IndexSignatureElement
        | K::ConstructSignatureElement => pick(IMAGE_ID_METHOD_PRIVATE, IMAGE_ID_METHOD_PROTECTED, IMAGE_ID_METHOD_PUBLIC),
        K::TypeParameterElement => new_vs_image_id(IMAGE_ID_TYPE),
        K::Label => new_vs_image_id(IMAGE_ID_LABEL),
        K::Alias => new_vs_image_id(IMAGE_ID_MODULE_PUBLIC),
        _ => new_vs_image_id(IMAGE_ID_ASSEMBLY),
    }
}

// buildVSHoverRawContent assembles the VS-specific rich hover content (symbol icon + colorized
// declaration line, plus an optional colorized documentation block) matching the shape that
// TypeScript-VS's legacy HoverService.cs builds from TSServer's quickinfo-full response
// (ImageElement + ClassifiedTextElement wrapped in a ContainerElement).
// hovericon.go:133
pub(crate) fn build_vs_hover_raw_content(
    image_id: lsproto::VSImageId,
    quick_info_runs: Vec<lsproto::VSClassifiedTextRun>,
    documentation_runs: Vec<lsproto::VSClassifiedTextRun>,
) -> Option<lsproto::VSContainerElement> {
    if quick_info_runs.is_empty() {
        return None;
    }

    let display_line = lsproto::VSContainerElement {
        style: lsproto::VSContainerElementStyle::Wrapped,
        elements: vec![
            lsproto::VSImageElementOrClassifiedTextElementOrContainerElement {
                image_element: Some(lsproto::VSImageElement { image_id, ..Default::default() }),
                ..Default::default()
            },
            lsproto::VSImageElementOrClassifiedTextElementOrContainerElement {
                classified_text_element: Some(lsproto::VSClassifiedTextElement { runs: quick_info_runs, ..Default::default() }),
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    if documentation_runs.is_empty() {
        return Some(display_line);
    }

    Some(lsproto::VSContainerElement {
        style: lsproto::VSContainerElementStyle::Stacked,
        elements: vec![
            lsproto::VSImageElementOrClassifiedTextElementOrContainerElement { container_element: Some(display_line), ..Default::default() },
            lsproto::VSImageElementOrClassifiedTextElementOrContainerElement {
                classified_text_element: Some(lsproto::VSClassifiedTextElement { runs: documentation_runs, ..Default::default() }),
                ..Default::default()
            },
        ],
        ..Default::default()
    })
}
