pub(super) fn is_rust_builtin(name: &str) -> bool {
    matches!(
        name,
        "Ok" | "Err"
            | "Some"
            | "None"
            | "Box"
            | "Vec"
            | "String"
            | "Default"
            | "From"
            | "Into"
            | "Clone"
            | "Drop"
    )
}

pub(super) fn is_python_builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "len"
            | "range"
            | "enumerate"
            | "zip"
            | "map"
            | "filter"
            | "sorted"
            | "reversed"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "str"
            | "int"
            | "float"
            | "bool"
            | "type"
            | "isinstance"
            | "hasattr"
            | "getattr"
            | "setattr"
            | "super"
            | "open"
            | "input"
            | "repr"
            | "abs"
            | "max"
            | "min"
            | "sum"
            | "any"
            | "all"
            | "iter"
            | "next"
            | "id"
            | "hash"
    )
}

pub(super) fn is_js_builtin(name: &str) -> bool {
    matches!(
        name,
        "require"
            | "import"
            | "console"
            | "setTimeout"
            | "setInterval"
            | "clearTimeout"
            | "clearInterval"
            | "Promise"
            | "Array"
            | "Object"
            | "String"
            | "Number"
            | "Boolean"
            | "Error"
            | "Map"
            | "Set"
            | "JSON"
            | "Math"
            | "Date"
            | "Symbol"
            | "parseInt"
            | "parseFloat"
            | "isNaN"
            | "fetch"
    )
}

pub(super) fn is_go_builtin(name: &str) -> bool {
    matches!(
        name,
        "make"
            | "new"
            | "len"
            | "cap"
            | "append"
            | "copy"
            | "delete"
            | "close"
            | "panic"
            | "recover"
            | "print"
            | "println"
    )
}

pub(super) fn is_c_builtin(name: &str) -> bool {
    matches!(
        name,
        "printf"
            | "fprintf"
            | "sprintf"
            | "snprintf"
            | "scanf"
            | "fscanf"
            | "malloc"
            | "calloc"
            | "realloc"
            | "free"
            | "memcpy"
            | "memmove"
            | "memset"
            | "memcmp"
            | "strlen"
            | "strcpy"
            | "strncpy"
            | "strcmp"
            | "strncmp"
            | "fopen"
            | "fclose"
            | "fread"
            | "fwrite"
            | "fgets"
            | "fputs"
            | "assert"
            | "exit"
            | "abort"
    )
}
