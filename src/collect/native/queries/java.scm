(package_declaration) @package
(import_declaration) @import
(class_declaration) @declaration
(interface_declaration) @declaration
(record_declaration) @declaration
(enum_declaration) @declaration
(annotation_type_declaration) @declaration
(type_identifier) @type
(scoped_type_identifier) @type
(type_parameter (type_identifier) @parameter)
(method_invocation object: (_) @receiver)
(field_access) @receiver
(method_reference . (identifier) @receiver)
(method_invocation name: (identifier) @member)
(identifier) @identifier
(marker_annotation name: (_) @annotation)
(annotation name: (_) @annotation)
(method_declaration) @method.declaration
(field_declaration) @field.declaration
(enum_constant) @enum.declaration
(module_declaration) @module
