//! Reads `wit/` and lowers it to the ABI model by the rules of `docs/abi-spec.md`.
//!
//! Any syntax outside the subset of docs/abi-spec.md §2 is an error.

use anyhow::{Context, Result, bail, ensure};
use wit_parser::{
    Docs, Function, FunctionKind, Handle, Interface, InterfaceId, Resolve, Type, TypeDefKind,
    WorldItem,
};

use crate::model::{
    EnumCase, EnumDef, FlagBit, FlagsDef, Hal, Iface, Import, Param, Ret, Role, Scalar,
};

/// Reads `wit/` and builds the ABI model.
pub fn load(wit_dir: &std::path::Path) -> Result<Hal> {
    let mut resolve = Resolve::default();
    let (pkg_id, _) = resolve
        .push_dir(wit_dir)
        .with_context(|| format!("cannot parse `{}`", wit_dir.display()))?;

    let pkg = &resolve.packages[pkg_id];
    let version = pkg
        .name
        .version
        .as_ref()
        .context("package has no version; docs/abi-spec.md §3.1 requires one")?;
    let package = format!("{}:{}@{}", pkg.name.namespace, pkg.name.name, version);

    let world_id = *pkg
        .worlds
        .get("app")
        .context("world `app` not found (docs/abi-spec.md §2.3)")?;
    let world = &resolve.worlds[world_id];

    let run = world
        .exports
        .iter()
        .find_map(|(_, item)| match item {
            WorldItem::Function(f) if f.name == "run" => Some(f),
            _ => None,
        })
        .context("world `app` does not export `run` (docs/abi-spec.md §3.3)")?;
    ensure!(
        run.params.is_empty() && run.result.is_none(),
        "export `run` must be `func() -> ()` (docs/abi-spec.md §3.3)"
    );

    // Interfaces keep the import order of `world app`.
    let mut interfaces = Vec::new();
    for (key, item) in &world.imports {
        let WorldItem::Interface { id, .. } = item else {
            bail!("import {key:?} of world `app` is not an interface (docs/abi-spec.md §2.3)");
        };
        let iface = &resolve.interfaces[*id];
        let name = iface
            .name
            .clone()
            .context("anonymous interfaces are not supported")?;
        let module = format!(
            "{}:{}/{}@{}",
            pkg.name.namespace, pkg.name.name, name, version
        );
        interfaces.push(
            lower_interface(&resolve, *id, iface, &name, module)
                .with_context(|| format!("cannot lower interface `{name}`"))?,
        );
    }

    Ok(Hal {
        package,
        interfaces,
    })
}

fn lower_interface(
    resolve: &Resolve,
    id: InterfaceId,
    iface: &Interface,
    name: &str,
    module: String,
) -> Result<Iface> {
    let mut enums = Vec::new();
    let mut flags = Vec::new();
    let mut resources = Vec::new();

    for (ty_name, ty_id) in &iface.types {
        let td = &resolve.types[*ty_id];
        match &td.kind {
            // Types brought in with `use` are defined in another interface; skip them.
            TypeDefKind::Type(_) => {}
            TypeDefKind::Resource => resources.push(ty_name.clone()),
            TypeDefKind::Enum(e) => enums.push(EnumDef {
                docs: docs(&td.docs),
                wit_name: ty_name.clone(),
                rust_name: pascal(ty_name),
                as_name: as_type_name(name, ty_name),
                cases: e
                    .cases
                    .iter()
                    .map(|c| EnumCase {
                        docs: docs(&c.docs),
                        wit_name: c.name.clone(),
                        ident: pascal(&c.name),
                    })
                    .collect(),
            }),
            TypeDefKind::Flags(f) => {
                ensure!(
                    f.flags.len() <= 32,
                    "flags `{ty_name}` has more than 32 cases (docs/abi-spec.md §2.1)"
                );
                flags.push(FlagsDef {
                    docs: docs(&td.docs),
                    wit_name: ty_name.clone(),
                    rust_name: pascal(ty_name),
                    as_name: as_type_name(name, ty_name),
                    bits: f
                        .flags
                        .iter()
                        .map(|b| FlagBit {
                            docs: docs(&b.docs),
                            wit_name: b.name.clone(),
                            ident: pascal(&b.name),
                        })
                        .collect(),
                });
            }
            other => bail!(
                "type `{ty_name}` of kind {} is outside the WIT subset (docs/abi-spec.md §2.2)",
                other.as_str()
            ),
        }
        if let TypeDefKind::Enum(e) = &td.kind {
            ensure!(
                e.cases.len() <= 256,
                "enum `{ty_name}` has more than 256 cases (docs/abi-spec.md §2.1)"
            );
        }
    }

    let mut imports = Vec::new();
    for (fn_name, func) in &iface.functions {
        imports.push(
            lower_function(resolve, id, name, fn_name, func)
                .with_context(|| format!("cannot lower function `{fn_name}`"))?,
        );
    }
    // The implicit drop goes after the declared functions (docs/abi-spec.md §3.2).
    for r in &resources {
        imports.push(Import {
            name: format!("[resource-drop]{r}"),
            docs: vec![format!(
                "Releases a `{r}` handle. Invalid handles are ignored (docs/abi-spec.md §5.2)."
            )],
            params: vec![Param {
                name: "self".to_string(),
                role: Role::Handle,
            }],
            ret: Ret::None,
            slot: format!("{}{}Drop", pascal(name), pascal(r)),
            rust_name: format!("{}_drop", snake(r)),
            as_name: format!("{}_{}_drop", snake(name), snake(r)),
        });
    }

    Ok(Iface {
        name: name.to_string(),
        module,
        docs: docs(&iface.docs),
        enums,
        flags,
        imports,
    })
}

fn lower_function(
    resolve: &Resolve,
    iface_id: InterfaceId,
    iface_name: &str,
    fn_name: &str,
    func: &Function,
) -> Result<Import> {
    match func.kind {
        FunctionKind::Freestanding | FunctionKind::Method(_) | FunctionKind::Static(_) => {}
        FunctionKind::Constructor(_) => {
            bail!(
                "constructors are outside the WIT subset; use a static `open` (docs/abi-spec.md §2.2)"
            )
        }
        _ => bail!("async functions are outside the WIT subset (docs/abi-spec.md §2.2)"),
    }

    let is_method = matches!(func.kind, FunctionKind::Method(_));
    let mut params = Vec::new();
    for (i, p) in func.params.iter().enumerate() {
        if i == 0 && is_method {
            // The first parameter of a method is the implicit borrowed handle.
            params.push(Param {
                name: "self".to_string(),
                role: Role::Handle,
            });
            continue;
        }
        lower_param(resolve, &p.name, p.ty, &mut params)
            .with_context(|| format!("cannot lower parameter `{}`", p.name))?;
    }

    let ret = match func.result {
        None => Ret::None,
        Some(ty) => match result_of(resolve, ty) {
            Some((ok, err)) => {
                ensure!(
                    err.is_some_and(|e| is_error_code(resolve, e)),
                    "the error type of `result` must be `types.error-code` (docs/abi-spec.md §4.4)"
                );
                if let Some(ok) = ok {
                    lower_out(resolve, ok, &mut params)?;
                }
                Ret::Status
            }
            None => Ret::Scalar(
                scalar_of(resolve, ty).context("return type is neither a scalar nor a `result`")?,
            ),
        },
    };

    // wit-parser already names functions `[static]pin.open`, the form of docs/abi-spec.md §3.2.
    let base = fn_name
        .strip_prefix("[static]")
        .or_else(|| fn_name.strip_prefix("[method]"))
        .unwrap_or(fn_name);
    let _ = iface_id;

    Ok(Import {
        name: fn_name.to_string(),
        docs: docs(&func.docs),
        params,
        ret,
        slot: format!("{}{}", pascal(iface_name), pascal(base)),
        rust_name: snake(base),
        as_name: format!("{}_{}", snake(iface_name), snake(base)),
    })
}

/// Lowers one parameter (docs/abi-spec.md §4.1, §4.2).
fn lower_param(resolve: &Resolve, name: &str, ty: Type, out: &mut Vec<Param>) -> Result<()> {
    if let Type::Id(id) = ty {
        match &resolve.types[id].kind {
            TypeDefKind::Handle(_) => {
                out.push(Param {
                    name: name.to_string(),
                    role: Role::Handle,
                });
                return Ok(());
            }
            TypeDefKind::List(elem) => {
                ensure!(
                    matches!(elem, Type::U8),
                    "lists other than `list<u8>` are outside the WIT subset (docs/abi-spec.md §2.2)"
                );
                out.push(Param {
                    name: format!("{name}-ptr"),
                    role: Role::DataPtr,
                });
                out.push(Param {
                    name: format!("{name}-len"),
                    role: Role::DataLen,
                });
                return Ok(());
            }
            TypeDefKind::Type(inner) => return lower_param(resolve, name, *inner, out),
            _ => {}
        }
    }
    if matches!(ty, Type::String) {
        out.push(Param {
            name: format!("{name}-ptr"),
            role: Role::StrPtr,
        });
        out.push(Param {
            name: format!("{name}-len"),
            role: Role::DataLen,
        });
        return Ok(());
    }
    let s = scalar_of(resolve, ty)
        .context("parameter type is outside the WIT subset (docs/abi-spec.md §2.2)")?;
    out.push(Param {
        name: name.to_string(),
        role: Role::Scalar(s),
    });
    Ok(())
}

/// Lowers the `T` of `result<T, _>` to a trailing out-pointer (docs/abi-spec.md §4.4).
fn lower_out(resolve: &Resolve, ty: Type, out: &mut Vec<Param>) -> Result<()> {
    if let Type::Id(id) = ty {
        match &resolve.types[id].kind {
            TypeDefKind::Handle(h) => {
                let _: &Handle = h;
                out.push(Param {
                    name: "out".to_string(),
                    role: Role::OutHandle,
                });
                return Ok(());
            }
            TypeDefKind::List(elem) => {
                ensure!(
                    matches!(elem, Type::U8),
                    "lists other than `list<u8>` are outside the WIT subset (docs/abi-spec.md §2.2)"
                );
                out.push(Param {
                    name: "buf".to_string(),
                    role: Role::OutBuf,
                });
                out.push(Param {
                    name: "cap".to_string(),
                    role: Role::OutCap,
                });
                out.push(Param {
                    name: "len-out".to_string(),
                    role: Role::OutLen,
                });
                return Ok(());
            }
            TypeDefKind::Type(inner) => return lower_out(resolve, *inner, out),
            TypeDefKind::Result(_) => {
                bail!("nested `result` is outside the WIT subset (docs/abi-spec.md §2.2)")
            }
            _ => {}
        }
    }
    ensure!(
        !matches!(ty, Type::String),
        "`string` return values are outside the WIT subset (docs/abi-spec.md §2.2)"
    );
    let s = scalar_of(resolve, ty)
        .context("the `T` of `result` is outside the WIT subset (docs/abi-spec.md §2.2)")?;
    out.push(Param {
        name: "out".to_string(),
        role: Role::OutScalar(s),
    });
    Ok(())
}

/// Returns `(ok, err)` if `ty` is a `result<..>`.
fn result_of(resolve: &Resolve, ty: Type) -> Option<(Option<Type>, Option<Type>)> {
    let Type::Id(id) = ty else { return None };
    match &resolve.types[id].kind {
        TypeDefKind::Result(r) => Some((r.ok, r.err)),
        TypeDefKind::Type(inner) => result_of(resolve, *inner),
        _ => None,
    }
}

/// Returns the `Scalar` of a scalar, enum or flags type.
fn scalar_of(resolve: &Resolve, ty: Type) -> Option<Scalar> {
    Some(match ty {
        Type::Bool => Scalar::Bool,
        Type::U8 => Scalar::U8,
        Type::U16 => Scalar::U16,
        Type::U32 => Scalar::U32,
        Type::S32 => Scalar::S32,
        Type::U64 => Scalar::U64,
        Type::S64 => Scalar::S64,
        Type::F32 => Scalar::F32,
        Type::F64 => Scalar::F64,
        Type::Id(id) => {
            let td = &resolve.types[id];
            let name = td.name.clone().unwrap_or_default();
            match &td.kind {
                TypeDefKind::Enum(_) => Scalar::Enum(name),
                TypeDefKind::Flags(_) => Scalar::Flags(name),
                TypeDefKind::Type(inner) => return scalar_of(resolve, *inner),
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn is_error_code(resolve: &Resolve, ty: Type) -> bool {
    let Type::Id(id) = ty else { return false };
    let td = &resolve.types[id];
    if let TypeDefKind::Type(inner) = &td.kind {
        return is_error_code(resolve, *inner);
    }
    matches!(td.kind, TypeDefKind::Enum(_)) && td.name.as_deref() == Some("error-code")
}

/// Splits a doc comment into lines (docs/abi-spec.md §2.3).
fn docs(d: &Docs) -> Vec<String> {
    d.contents
        .as_deref()
        .map(|c| c.lines().map(|l| l.trim_end().to_string()).collect())
        .unwrap_or_default()
}

/// `pin-mode` / `pin.open` -> `PinMode` / `PinOpen`.
pub fn pascal(s: &str) -> String {
    s.split(['-', '.'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// `pin-mode` / `pin.open` -> `pin_mode` / `pin_open`.
pub fn snake(s: &str) -> String {
    s.replace(['-', '.'], "_")
}

/// Prefixes a type name with its interface name, because the AssemblyScript binding has no
/// namespaces. Only `types`, which holds the shared types, stays unprefixed.
fn as_type_name(iface: &str, ty: &str) -> String {
    if iface == "types" {
        pascal(ty)
    } else {
        format!("{}{}", pascal(iface), pascal(ty))
    }
}
