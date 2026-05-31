import json
import sys


with open(sys.argv[1], encoding="utf-8") as f:
    data = json.load(f)

index = {int(k): v for k, v in data["index"].items()}
local_paths = {int(k): v for k, v in data["paths"].items() if v["crate_id"] == 0}
root = int(data["root"])


def item_kind(item: dict) -> str:
    return next(iter(item["inner"]))


def local_path(item_id: int) -> str:
    if item_id in local_paths:
        return "::".join(local_paths[item_id]["path"])
    item = index[item_id]
    if item_id == root:
        return item["name"]
    return item.get("name") or f"<unnamed:{item_id}>"


def is_ignored_local_path(path: str) -> bool:
    return path in {"demo::__builtin_va_list", "demo::__gnuc_va_list"}


def summarize_type(ty: dict | None):
    if ty is None:
        return None
    if "primitive" in ty:
        return {"primitive": ty["primitive"]}
    if "raw_pointer" in ty:
        pointer = ty["raw_pointer"]
        return {
            "raw_pointer": {
                "is_mutable": pointer["is_mutable"],
                "type": summarize_type(pointer["type"]),
            }
        }
    if "resolved_path" in ty:
        resolved = ty["resolved_path"]
        result = {"path": resolved["path"]}
        resolved_id = resolved.get("id")
        if (
            isinstance(resolved_id, int)
            and resolved_id in index
            and index[resolved_id]["crate_id"] == 0
        ):
            result["id"] = (
                local_path(resolved_id)
                if resolved_id in local_paths
                else resolved["path"]
            )
        else:
            result["id"] = resolved_id
        if resolved.get("args") is not None:
            result["args"] = resolved["args"]
        return {"resolved_path": result}
    return ty


def summarize_field(item_id: int) -> dict:
    item = index[item_id]
    return {
        "name": item["name"],
        "kind": item_kind(item),
        "type": summarize_type(item["inner"]["struct_field"]),
    }


def summarize_item(item_id: int) -> dict:
    item = index[item_id]
    kind = item_kind(item)
    inner = item["inner"][kind]
    summary = {
        "path": local_path(item_id),
        "kind": kind,
        "visibility": item["visibility"],
        "filename": item.get("span", {}).get("filename"),
    }
    if item.get("docs") is not None:
        summary["docs"] = item["docs"]

    if kind == "module":
        summary["items"] = sorted(
            local_path(child_id)
            for child_id in inner["items"]
            if child_id in index and not is_ignored_local_path(local_path(child_id))
        )
    elif kind == "type_alias":
        summary["type"] = summarize_type(inner["type"])
    elif kind == "static":
        summary["type"] = summarize_type(inner["type"])
        summary["is_mutable"] = inner["is_mutable"]
        summary["is_unsafe"] = inner["is_unsafe"]
    elif kind == "function":
        summary["sig"] = {
            "inputs": [summarize_type(arg) for arg in inner["sig"]["inputs"]],
            "output": summarize_type(inner["sig"]["output"]),
            "is_c_variadic": inner["sig"]["is_c_variadic"],
        }
        summary["header"] = inner["header"]
        summary["has_body"] = inner["has_body"]
    elif kind == "struct":
        fields = inner["kind"]["plain"]["fields"]
        summary["fields"] = [summarize_field(field_id) for field_id in fields]
    elif kind == "union":
        summary["fields"] = [summarize_field(field_id) for field_id in inner["fields"]]

    return summary


items = []
for item_id, path_info in sorted(local_paths.items(), key=lambda pair: tuple(pair[1]["path"])):
    path = "::".join(path_info["path"])
    if is_ignored_local_path(path):
        continue
    items.append(summarize_item(item_id))

normalized = {
    "crate_version": data["crate_version"],
    "includes_private": data["includes_private"],
    "items": items,
}

json.dump(normalized, sys.stdout, indent=2)
sys.stdout.write("\n")
