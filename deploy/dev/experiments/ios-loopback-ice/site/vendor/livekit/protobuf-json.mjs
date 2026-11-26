// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 1960-2899 行按职责切出，行为不变。
import { assert, assertFloat32, assertInt32, assertUInt32 } from './protobuf-assert.mjs';
import { protoInt64 } from './protobuf-int64.mjs';
import { LongType, ScalarType, isScalarZeroValue, scalarZeroValue } from './protobuf-scalars.mjs';
import { _slicedToArray } from './tslib.mjs';
import { BinaryReader, BinaryWriter, WireType, createExtensionContainer, protoBase64 } from './protobuf-wire.mjs';
import { clearField, getExtension, hasExtension, isFieldSet, isMessage, setExtension, wrapField } from './protobuf-extensions.mjs';
import { writePacked, writeScalar } from './protobuf-codegen.mjs';


({
  "google.protobuf.DoubleValue": ScalarType.DOUBLE,
  "google.protobuf.FloatValue": ScalarType.FLOAT,
  "google.protobuf.Int64Value": ScalarType.INT64,
  "google.protobuf.UInt64Value": ScalarType.UINT64,
  "google.protobuf.Int32Value": ScalarType.INT32,
  "google.protobuf.UInt32Value": ScalarType.UINT32,
  "google.protobuf.BoolValue": ScalarType.BOOL,
  "google.protobuf.StringValue": ScalarType.STRING,
  "google.protobuf.BytesValue": ScalarType.BYTES
});/* eslint-disable no-case-declarations,@typescript-eslint/no-unsafe-argument,@typescript-eslint/no-unsafe-assignment,@typescript-eslint/no-unsafe-member-access,@typescript-eslint/no-unsafe-call */
// Default options for parsing JSON.
const jsonReadDefaults = {
  ignoreUnknownFields: false
};
// Default options for serializing to JSON.
const jsonWriteDefaults = {
  emitDefaultValues: false,
  enumAsInteger: false,
  useProtoFieldName: false,
  prettySpaces: 0
};
function makeReadOptions$1(options) {
  return options ? Object.assign(Object.assign({}, jsonReadDefaults), options) : jsonReadDefaults;
}
function makeWriteOptions$1(options) {
  return options ? Object.assign(Object.assign({}, jsonWriteDefaults), options) : jsonWriteDefaults;
}
const tokenNull = Symbol();
const tokenIgnoredUnknownEnum = Symbol();
function makeJsonFormat() {
  return {
    makeReadOptions: makeReadOptions$1,
    makeWriteOptions: makeWriteOptions$1,
    readMessage(type, json, options, message) {
      if (json == null || Array.isArray(json) || typeof json != "object") {
        throw new Error("cannot decode message ".concat(type.typeName, " from JSON: ").concat(debugJsonValue(json)));
      }
      message = message !== null && message !== void 0 ? message : new type();
      const oneofSeen = new Map();
      const registry = options.typeRegistry;
      for (const _ref of Object.entries(json)) {
        var _ref2 = _slicedToArray(_ref, 2);
        const jsonKey = _ref2[0];
        const jsonValue = _ref2[1];
        const field = type.fields.findJsonName(jsonKey);
        if (field) {
          if (field.oneof) {
            if (jsonValue === null && field.kind == "scalar") {
              // see conformance test Required.Proto3.JsonInput.OneofFieldNull{First,Second}
              continue;
            }
            const seen = oneofSeen.get(field.oneof);
            if (seen !== undefined) {
              throw new Error("cannot decode message ".concat(type.typeName, " from JSON: multiple keys for oneof \"").concat(field.oneof.name, "\" present: \"").concat(seen, "\", \"").concat(jsonKey, "\""));
            }
            oneofSeen.set(field.oneof, jsonKey);
          }
          readField$1(message, jsonValue, field, options, type);
        } else {
          let found = false;
          if ((registry === null || registry === void 0 ? void 0 : registry.findExtension) && jsonKey.startsWith("[") && jsonKey.endsWith("]")) {
            const ext = registry.findExtension(jsonKey.substring(1, jsonKey.length - 1));
            if (ext && ext.extendee.typeName == type.typeName) {
              found = true;
              const _createExtensionConta = createExtensionContainer(ext),
                _createExtensionConta2 = _slicedToArray(_createExtensionConta, 2),
                container = _createExtensionConta2[0],
                get = _createExtensionConta2[1];
              readField$1(container, jsonValue, ext.field, options, ext);
              // We pass on the options as BinaryReadOptions/BinaryWriteOptions,
              // so that users can bring their own binary reader and writer factories
              // if necessary.
              setExtension(message, ext, get(), options);
            }
          }
          if (!found && !options.ignoreUnknownFields) {
            throw new Error("cannot decode message ".concat(type.typeName, " from JSON: key \"").concat(jsonKey, "\" is unknown"));
          }
        }
      }
      return message;
    },
    writeMessage(message, options) {
      const type = message.getType();
      const json = {};
      let field;
      try {
        for (field of type.fields.byNumber()) {
          if (!isFieldSet(field, message)) {
            // eslint-disable-next-line @typescript-eslint/strict-boolean-expressions
            if (field.req) {
              throw "required field not set";
            }
            if (!options.emitDefaultValues) {
              continue;
            }
            if (!canEmitFieldDefaultValue(field)) {
              continue;
            }
          }
          const value = field.oneof ? message[field.oneof.localName].value : message[field.localName];
          const jsonValue = writeField$1(field, value, options);
          if (jsonValue !== undefined) {
            json[options.useProtoFieldName ? field.name : field.jsonName] = jsonValue;
          }
        }
        const registry = options.typeRegistry;
        if (registry === null || registry === void 0 ? void 0 : registry.findExtensionFor) {
          for (const uf of type.runtime.bin.listUnknownFields(message)) {
            const ext = registry.findExtensionFor(type.typeName, uf.no);
            if (ext && hasExtension(message, ext)) {
              // We pass on the options as BinaryReadOptions, so that users can bring their own
              // binary reader factory if necessary.
              const value = getExtension(message, ext, options);
              const jsonValue = writeField$1(ext.field, value, options);
              if (jsonValue !== undefined) {
                json[ext.field.jsonName] = jsonValue;
              }
            }
          }
        }
      } catch (e) {
        const m = field ? "cannot encode field ".concat(type.typeName, ".").concat(field.name, " to JSON") : "cannot encode message ".concat(type.typeName, " to JSON");
        const r = e instanceof Error ? e.message : String(e);
        throw new Error(m + (r.length > 0 ? ": ".concat(r) : ""));
      }
      return json;
    },
    readScalar(type, json, longType) {
      // The signature of our internal function has changed. For backwards-
      // compatibility, we support the old form that is part of the public API
      // through the interface JsonFormat.
      return readScalar$1(type, json, longType !== null && longType !== void 0 ? longType : LongType.BIGINT, true);
    },
    writeScalar(type, value, emitDefaultValues) {
      // The signature of our internal function has changed. For backwards-
      // compatibility, we support the old form that is part of the public API
      // through the interface JsonFormat.
      if (value === undefined) {
        return undefined;
      }
      if (emitDefaultValues || isScalarZeroValue(type, value)) {
        return writeScalar$1(type, value);
      }
      return undefined;
    },
    debug: debugJsonValue
  };
}
function debugJsonValue(json) {
  if (json === null) {
    return "null";
  }
  switch (typeof json) {
    case "object":
      return Array.isArray(json) ? "array" : "object";
    case "string":
      return json.length > 100 ? "string" : "\"".concat(json.split('"').join('\\"'), "\"");
    default:
      return String(json);
  }
}
// Read a JSON value for a field.
// The "parentType" argument is only used to provide context in errors.
function readField$1(target, jsonValue, field, options, parentType) {
  let localName = field.localName;
  if (field.repeated) {
    assert(field.kind != "map");
    if (jsonValue === null) {
      return;
    }
    if (!Array.isArray(jsonValue)) {
      throw new Error("cannot decode field ".concat(parentType.typeName, ".").concat(field.name, " from JSON: ").concat(debugJsonValue(jsonValue)));
    }
    const targetArray = target[localName];
    for (const jsonItem of jsonValue) {
      if (jsonItem === null) {
        throw new Error("cannot decode field ".concat(parentType.typeName, ".").concat(field.name, " from JSON: ").concat(debugJsonValue(jsonItem)));
      }
      switch (field.kind) {
        case "message":
          targetArray.push(field.T.fromJson(jsonItem, options));
          break;
        case "enum":
          const enumValue = readEnum(field.T, jsonItem, options.ignoreUnknownFields, true);
          if (enumValue !== tokenIgnoredUnknownEnum) {
            targetArray.push(enumValue);
          }
          break;
        case "scalar":
          try {
            targetArray.push(readScalar$1(field.T, jsonItem, field.L, true));
          } catch (e) {
            let m = "cannot decode field ".concat(parentType.typeName, ".").concat(field.name, " from JSON: ").concat(debugJsonValue(jsonItem));
            if (e instanceof Error && e.message.length > 0) {
              m += ": ".concat(e.message);
            }
            throw new Error(m);
          }
          break;
      }
    }
  } else if (field.kind == "map") {
    if (jsonValue === null) {
      return;
    }
    if (typeof jsonValue != "object" || Array.isArray(jsonValue)) {
      throw new Error("cannot decode field ".concat(parentType.typeName, ".").concat(field.name, " from JSON: ").concat(debugJsonValue(jsonValue)));
    }
    const targetMap = target[localName];
    for (const _ref3 of Object.entries(jsonValue)) {
      var _ref4 = _slicedToArray(_ref3, 2);
      const jsonMapKey = _ref4[0];
      const jsonMapValue = _ref4[1];
      if (jsonMapValue === null) {
        throw new Error("cannot decode field ".concat(parentType.typeName, ".").concat(field.name, " from JSON: map value null"));
      }
      let key;
      try {
        key = readMapKey(field.K, jsonMapKey);
      } catch (e) {
        let m = "cannot decode map key for field ".concat(parentType.typeName, ".").concat(field.name, " from JSON: ").concat(debugJsonValue(jsonValue));
        if (e instanceof Error && e.message.length > 0) {
          m += ": ".concat(e.message);
        }
        throw new Error(m);
      }
      switch (field.V.kind) {
        case "message":
          targetMap[key] = field.V.T.fromJson(jsonMapValue, options);
          break;
        case "enum":
          const enumValue = readEnum(field.V.T, jsonMapValue, options.ignoreUnknownFields, true);
          if (enumValue !== tokenIgnoredUnknownEnum) {
            targetMap[key] = enumValue;
          }
          break;
        case "scalar":
          try {
            targetMap[key] = readScalar$1(field.V.T, jsonMapValue, LongType.BIGINT, true);
          } catch (e) {
            let m = "cannot decode map value for field ".concat(parentType.typeName, ".").concat(field.name, " from JSON: ").concat(debugJsonValue(jsonValue));
            if (e instanceof Error && e.message.length > 0) {
              m += ": ".concat(e.message);
            }
            throw new Error(m);
          }
          break;
      }
    }
  } else {
    if (field.oneof) {
      target = target[field.oneof.localName] = {
        case: localName
      };
      localName = "value";
    }
    switch (field.kind) {
      case "message":
        const messageType = field.T;
        if (jsonValue === null && messageType.typeName != "google.protobuf.Value") {
          return;
        }
        let currentValue = target[localName];
        if (isMessage(currentValue)) {
          currentValue.fromJson(jsonValue, options);
        } else {
          target[localName] = currentValue = messageType.fromJson(jsonValue, options);
          if (messageType.fieldWrapper && !field.oneof) {
            target[localName] = messageType.fieldWrapper.unwrapField(currentValue);
          }
        }
        break;
      case "enum":
        const enumValue = readEnum(field.T, jsonValue, options.ignoreUnknownFields, false);
        switch (enumValue) {
          case tokenNull:
            clearField(field, target);
            break;
          case tokenIgnoredUnknownEnum:
            break;
          default:
            target[localName] = enumValue;
            break;
        }
        break;
      case "scalar":
        try {
          const scalarValue = readScalar$1(field.T, jsonValue, field.L, false);
          switch (scalarValue) {
            case tokenNull:
              clearField(field, target);
              break;
            default:
              target[localName] = scalarValue;
              break;
          }
        } catch (e) {
          let m = "cannot decode field ".concat(parentType.typeName, ".").concat(field.name, " from JSON: ").concat(debugJsonValue(jsonValue));
          if (e instanceof Error && e.message.length > 0) {
            m += ": ".concat(e.message);
          }
          throw new Error(m);
        }
        break;
    }
  }
}
function readMapKey(type, json) {
  if (type === ScalarType.BOOL) {
    // eslint-disable-next-line @typescript-eslint/switch-exhaustiveness-check
    switch (json) {
      case "true":
        json = true;
        break;
      case "false":
        json = false;
        break;
    }
  }
  return readScalar$1(type, json, LongType.BIGINT, true).toString();
}
function readScalar$1(type, json, longType, nullAsZeroValue) {
  if (json === null) {
    if (nullAsZeroValue) {
      return scalarZeroValue(type, longType);
    }
    return tokenNull;
  }
  // every valid case in the switch below returns, and every fall
  // through is regarded as a failure.
  switch (type) {
    // float, double: JSON value will be a number or one of the special string values "NaN", "Infinity", and "-Infinity".
    // Either numbers or strings are accepted. Exponent notation is also accepted.
    case ScalarType.DOUBLE:
    case ScalarType.FLOAT:
      if (json === "NaN") return Number.NaN;
      if (json === "Infinity") return Number.POSITIVE_INFINITY;
      if (json === "-Infinity") return Number.NEGATIVE_INFINITY;
      if (json === "") {
        // empty string is not a number
        break;
      }
      if (typeof json == "string" && json.trim().length !== json.length) {
        // extra whitespace
        break;
      }
      if (typeof json != "string" && typeof json != "number") {
        break;
      }
      const float = Number(json);
      if (Number.isNaN(float)) {
        // not a number
        break;
      }
      if (!Number.isFinite(float)) {
        // infinity and -infinity are handled by string representation above, so this is an error
        break;
      }
      if (type == ScalarType.FLOAT) assertFloat32(float);
      return float;
    // int32, fixed32, uint32: JSON value will be a decimal number. Either numbers or strings are accepted.
    case ScalarType.INT32:
    case ScalarType.FIXED32:
    case ScalarType.SFIXED32:
    case ScalarType.SINT32:
    case ScalarType.UINT32:
      let int32;
      if (typeof json == "number") int32 = json;else if (typeof json == "string" && json.length > 0) {
        if (json.trim().length === json.length) int32 = Number(json);
      }
      if (int32 === undefined) break;
      if (type == ScalarType.UINT32 || type == ScalarType.FIXED32) assertUInt32(int32);else assertInt32(int32);
      return int32;
    // int64, fixed64, uint64: JSON value will be a decimal string. Either numbers or strings are accepted.
    case ScalarType.INT64:
    case ScalarType.SFIXED64:
    case ScalarType.SINT64:
      if (typeof json != "number" && typeof json != "string") break;
      const long = protoInt64.parse(json);
      // eslint-disable-next-line @typescript-eslint/strict-boolean-expressions
      return longType ? long.toString() : long;
    case ScalarType.FIXED64:
    case ScalarType.UINT64:
      if (typeof json != "number" && typeof json != "string") break;
      const uLong = protoInt64.uParse(json);
      // eslint-disable-next-line @typescript-eslint/strict-boolean-expressions
      return longType ? uLong.toString() : uLong;
    // bool:
    case ScalarType.BOOL:
      if (typeof json !== "boolean") break;
      return json;
    // string:
    case ScalarType.STRING:
      if (typeof json !== "string") {
        break;
      }
      // A string must always contain UTF-8 encoded or 7-bit ASCII.
      // We validate with encodeURIComponent, which appears to be the fastest widely available option.
      try {
        encodeURIComponent(json);
      } catch (e) {
        throw new Error("invalid UTF8");
      }
      return json;
    // bytes: JSON value will be the data encoded as a string using standard base64 encoding with paddings.
    // Either standard or URL-safe base64 encoding with/without paddings are accepted.
    case ScalarType.BYTES:
      if (json === "") return new Uint8Array(0);
      if (typeof json !== "string") break;
      return protoBase64.dec(json);
  }
  throw new Error();
}
function readEnum(type, json, ignoreUnknownFields, nullAsZeroValue) {
  if (json === null) {
    if (type.typeName == "google.protobuf.NullValue") {
      return 0; // google.protobuf.NullValue.NULL_VALUE = 0
    }
    return nullAsZeroValue ? type.values[0].no : tokenNull;
  }
  // eslint-disable-next-line @typescript-eslint/switch-exhaustiveness-check
  switch (typeof json) {
    case "number":
      if (Number.isInteger(json)) {
        return json;
      }
      break;
    case "string":
      const value = type.findName(json);
      if (value !== undefined) {
        return value.no;
      }
      if (ignoreUnknownFields) {
        return tokenIgnoredUnknownEnum;
      }
      break;
  }
  throw new Error("cannot decode enum ".concat(type.typeName, " from JSON: ").concat(debugJsonValue(json)));
}
// Decide whether an unset field should be emitted with JSON write option `emitDefaultValues`
function canEmitFieldDefaultValue(field) {
  if (field.repeated || field.kind == "map") {
    // maps are {}, repeated fields are []
    return true;
  }
  if (field.oneof) {
    // oneof fields are never emitted
    return false;
  }
  if (field.kind == "message") {
    // singular message field are allowed to emit JSON null, but we do not
    return false;
  }
  // eslint-disable-next-line @typescript-eslint/strict-boolean-expressions
  if (field.opt || field.req) {
    // the field uses explicit presence, so we cannot emit a zero value
    return false;
  }
  return true;
}
function writeField$1(field, value, options) {
  if (field.kind == "map") {
    assert(typeof value == "object" && value != null);
    const jsonObj = {};
    const entries = Object.entries(value);
    switch (field.V.kind) {
      case "scalar":
        for (const _ref5 of entries) {
          var _ref6 = _slicedToArray(_ref5, 2);
          const entryKey = _ref6[0];
          const entryValue = _ref6[1];
          jsonObj[entryKey.toString()] = writeScalar$1(field.V.T, entryValue); // JSON standard allows only (double quoted) string as property key
        }
        break;
      case "message":
        for (const _ref7 of entries) {
          var _ref8 = _slicedToArray(_ref7, 2);
          const entryKey = _ref8[0];
          const entryValue = _ref8[1];
          // JSON standard allows only (double quoted) string as property key
          jsonObj[entryKey.toString()] = entryValue.toJson(options);
        }
        break;
      case "enum":
        const enumType = field.V.T;
        for (const _ref9 of entries) {
          var _ref0 = _slicedToArray(_ref9, 2);
          const entryKey = _ref0[0];
          const entryValue = _ref0[1];
          // JSON standard allows only (double quoted) string as property key
          jsonObj[entryKey.toString()] = writeEnum(enumType, entryValue, options.enumAsInteger);
        }
        break;
    }
    return options.emitDefaultValues || entries.length > 0 ? jsonObj : undefined;
  }
  if (field.repeated) {
    assert(Array.isArray(value));
    const jsonArr = [];
    switch (field.kind) {
      case "scalar":
        for (let i = 0; i < value.length; i++) {
          jsonArr.push(writeScalar$1(field.T, value[i]));
        }
        break;
      case "enum":
        for (let i = 0; i < value.length; i++) {
          jsonArr.push(writeEnum(field.T, value[i], options.enumAsInteger));
        }
        break;
      case "message":
        for (let i = 0; i < value.length; i++) {
          jsonArr.push(value[i].toJson(options));
        }
        break;
    }
    return options.emitDefaultValues || jsonArr.length > 0 ? jsonArr : undefined;
  }
  switch (field.kind) {
    case "scalar":
      return writeScalar$1(field.T, value);
    case "enum":
      return writeEnum(field.T, value, options.enumAsInteger);
    case "message":
      return wrapField(field.T, value).toJson(options);
  }
}
function writeEnum(type, value, enumAsInteger) {
  var _a;
  assert(typeof value == "number");
  if (type.typeName == "google.protobuf.NullValue") {
    return null;
  }
  if (enumAsInteger) {
    return value;
  }
  const val = type.findNumber(value);
  return (_a = val === null || val === void 0 ? void 0 : val.name) !== null && _a !== void 0 ? _a : value; // if we don't know the enum value, just return the number
}
function writeScalar$1(type, value) {
  switch (type) {
    // int32, fixed32, uint32: JSON value will be a decimal number. Either numbers or strings are accepted.
    case ScalarType.INT32:
    case ScalarType.SFIXED32:
    case ScalarType.SINT32:
    case ScalarType.FIXED32:
    case ScalarType.UINT32:
      assert(typeof value == "number");
      return value;
    // float, double: JSON value will be a number or one of the special string values "NaN", "Infinity", and "-Infinity".
    // Either numbers or strings are accepted. Exponent notation is also accepted.
    case ScalarType.FLOAT:
    // assertFloat32(value);
    case ScalarType.DOUBLE:
      // eslint-disable-line no-fallthrough
      assert(typeof value == "number");
      if (Number.isNaN(value)) return "NaN";
      if (value === Number.POSITIVE_INFINITY) return "Infinity";
      if (value === Number.NEGATIVE_INFINITY) return "-Infinity";
      return value;
    // string:
    case ScalarType.STRING:
      assert(typeof value == "string");
      return value;
    // bool:
    case ScalarType.BOOL:
      assert(typeof value == "boolean");
      return value;
    // JSON value will be a decimal string. Either numbers or strings are accepted.
    case ScalarType.UINT64:
    case ScalarType.FIXED64:
    case ScalarType.INT64:
    case ScalarType.SFIXED64:
    case ScalarType.SINT64:
      assert(typeof value == "bigint" || typeof value == "string" || typeof value == "number");
      return value.toString();
    // bytes: JSON value will be the data encoded as a string using standard base64 encoding with paddings.
    // Either standard or URL-safe base64 encoding with/without paddings are accepted.
    case ScalarType.BYTES:
      assert(value instanceof Uint8Array);
      return protoBase64.enc(value);
  }
}/* eslint-disable prefer-const,no-case-declarations,@typescript-eslint/no-explicit-any,@typescript-eslint/no-unsafe-argument,@typescript-eslint/no-unsafe-assignment,@typescript-eslint/no-unsafe-member-access,@typescript-eslint/no-unsafe-call,@typescript-eslint/no-unsafe-return */
const unknownFieldsSymbol = Symbol("@bufbuild/protobuf/unknown-fields");
// Default options for parsing binary data.
const readDefaults = {
  readUnknownFields: true,
  readerFactory: bytes => new BinaryReader(bytes)
};
// Default options for serializing binary data.
const writeDefaults = {
  writeUnknownFields: true,
  writerFactory: () => new BinaryWriter()
};
function makeReadOptions(options) {
  return options ? Object.assign(Object.assign({}, readDefaults), options) : readDefaults;
}
function makeWriteOptions(options) {
  return options ? Object.assign(Object.assign({}, writeDefaults), options) : writeDefaults;
}
function makeBinaryFormat() {
  return {
    makeReadOptions,
    makeWriteOptions,
    listUnknownFields(message) {
      var _a;
      return (_a = message[unknownFieldsSymbol]) !== null && _a !== void 0 ? _a : [];
    },
    discardUnknownFields(message) {
      delete message[unknownFieldsSymbol];
    },
    writeUnknownFields(message, writer) {
      const m = message;
      const c = m[unknownFieldsSymbol];
      if (c) {
        for (const f of c) {
          writer.tag(f.no, f.wireType).raw(f.data);
        }
      }
    },
    onUnknownField(message, no, wireType, data) {
      const m = message;
      if (!Array.isArray(m[unknownFieldsSymbol])) {
        m[unknownFieldsSymbol] = [];
      }
      m[unknownFieldsSymbol].push({
        no,
        wireType,
        data
      });
    },
    readMessage(message, reader, lengthOrEndTagFieldNo, options, delimitedMessageEncoding) {
      const type = message.getType();
      // eslint-disable-next-line @typescript-eslint/strict-boolean-expressions
      const end = delimitedMessageEncoding ? reader.len : reader.pos + lengthOrEndTagFieldNo;
      let fieldNo, wireType;
      while (reader.pos < end) {
        var _reader$tag = reader.tag();
        var _reader$tag2 = _slicedToArray(_reader$tag, 2);
        fieldNo = _reader$tag2[0];
        wireType = _reader$tag2[1];
        if (delimitedMessageEncoding === true && wireType == WireType.EndGroup) {
          break;
        }
        const field = type.fields.find(fieldNo);
        if (!field) {
          const data = reader.skip(wireType, fieldNo);
          if (options.readUnknownFields) {
            this.onUnknownField(message, fieldNo, wireType, data);
          }
          continue;
        }
        readField(message, reader, field, wireType, options);
      }
      if (delimitedMessageEncoding && (
      // eslint-disable-line @typescript-eslint/strict-boolean-expressions
      wireType != WireType.EndGroup || fieldNo !== lengthOrEndTagFieldNo)) {
        throw new Error("invalid end group tag");
      }
    },
    readField,
    writeMessage(message, writer, options) {
      const type = message.getType();
      for (const field of type.fields.byNumber()) {
        if (!isFieldSet(field, message)) {
          if (field.req) {
            throw new Error("cannot encode field ".concat(type.typeName, ".").concat(field.name, " to binary: required field not set"));
          }
          continue;
        }
        const value = field.oneof ? message[field.oneof.localName].value : message[field.localName];
        writeField(field, value, writer, options);
      }
      if (options.writeUnknownFields) {
        this.writeUnknownFields(message, writer);
      }
      return writer;
    },
    writeField(field, value, writer, options) {
      // The behavior of our internal function has changed, it does no longer
      // accept `undefined` values for singular scalar and map.
      // For backwards-compatibility, we support the old form that is part of
      // the public API through the interface BinaryFormat.
      if (value === undefined) {
        return undefined;
      }
      writeField(field, value, writer, options);
    }
  };
}
function readField(target,
// eslint-disable-line @typescript-eslint/no-explicit-any -- `any` is the best choice for dynamic access
reader, field, wireType, options) {
  let repeated = field.repeated,
    localName = field.localName;
  if (field.oneof) {
    target = target[field.oneof.localName];
    if (target.case != localName) {
      delete target.value;
    }
    target.case = localName;
    localName = "value";
  }
  switch (field.kind) {
    case "scalar":
    case "enum":
      const scalarType = field.kind == "enum" ? ScalarType.INT32 : field.T;
      let read = readScalar;
      // eslint-disable-next-line @typescript-eslint/no-unsafe-enum-comparison -- acceptable since it's covered by tests
      if (field.kind == "scalar" && field.L > 0) {
        read = readScalarLTString;
      }
      if (repeated) {
        let arr = target[localName]; // safe to assume presence of array, oneof cannot contain repeated values
        const isPacked = wireType == WireType.LengthDelimited && scalarType != ScalarType.STRING && scalarType != ScalarType.BYTES;
        if (isPacked) {
          let e = reader.uint32() + reader.pos;
          while (reader.pos < e) {
            arr.push(read(reader, scalarType));
          }
        } else {
          arr.push(read(reader, scalarType));
        }
      } else {
        target[localName] = read(reader, scalarType);
      }
      break;
    case "message":
      const messageType = field.T;
      if (repeated) {
        // safe to assume presence of array, oneof cannot contain repeated values
        target[localName].push(readMessageField(reader, new messageType(), options, field));
      } else {
        if (isMessage(target[localName])) {
          readMessageField(reader, target[localName], options, field);
        } else {
          target[localName] = readMessageField(reader, new messageType(), options, field);
          if (messageType.fieldWrapper && !field.oneof && !field.repeated) {
            target[localName] = messageType.fieldWrapper.unwrapField(target[localName]);
          }
        }
      }
      break;
    case "map":
      let _readMapEntry = readMapEntry(field, reader, options),
        _readMapEntry2 = _slicedToArray(_readMapEntry, 2),
        mapKey = _readMapEntry2[0],
        mapVal = _readMapEntry2[1];
      // safe to assume presence of map object, oneof cannot contain repeated values
      target[localName][mapKey] = mapVal;
      break;
  }
}
// Read a message, avoiding MessageType.fromBinary() to re-use the
// BinaryReadOptions and the IBinaryReader.
function readMessageField(reader, message, options, field) {
  const format = message.getType().runtime.bin;
  const delimited = field === null || field === void 0 ? void 0 : field.delimited;
  format.readMessage(message, reader, delimited ? field.no : reader.uint32(),
  // eslint-disable-line @typescript-eslint/strict-boolean-expressions
  options, delimited);
  return message;
}
// Read a map field, expecting key field = 1, value field = 2
function readMapEntry(field, reader, options) {
  const length = reader.uint32(),
    end = reader.pos + length;
  let key, val;
  while (reader.pos < end) {
    const _reader$tag3 = reader.tag(),
      _reader$tag4 = _slicedToArray(_reader$tag3, 1),
      fieldNo = _reader$tag4[0];
    switch (fieldNo) {
      case 1:
        key = readScalar(reader, field.K);
        break;
      case 2:
        switch (field.V.kind) {
          case "scalar":
            val = readScalar(reader, field.V.T);
            break;
          case "enum":
            val = reader.int32();
            break;
          case "message":
            val = readMessageField(reader, new field.V.T(), options, undefined);
            break;
        }
        break;
    }
  }
  if (key === undefined) {
    key = scalarZeroValue(field.K, LongType.BIGINT);
  }
  if (typeof key != "string" && typeof key != "number") {
    key = key.toString();
  }
  if (val === undefined) {
    switch (field.V.kind) {
      case "scalar":
        val = scalarZeroValue(field.V.T, LongType.BIGINT);
        break;
      case "enum":
        val = field.V.T.values[0].no;
        break;
      case "message":
        val = new field.V.T();
        break;
    }
  }
  return [key, val];
}
// Read a scalar value, but return 64 bit integral types (int64, uint64,
// sint64, fixed64, sfixed64) as string instead of bigint.
function readScalarLTString(reader, type) {
  const v = readScalar(reader, type);
  return typeof v == "bigint" ? v.toString() : v;
}
// Does not use scalarTypeInfo() for better performance.
function readScalar(reader, type) {
  switch (type) {
    case ScalarType.STRING:
      return reader.string();
    case ScalarType.BOOL:
      return reader.bool();
    case ScalarType.DOUBLE:
      return reader.double();
    case ScalarType.FLOAT:
      return reader.float();
    case ScalarType.INT32:
      return reader.int32();
    case ScalarType.INT64:
      return reader.int64();
    case ScalarType.UINT64:
      return reader.uint64();
    case ScalarType.FIXED64:
      return reader.fixed64();
    case ScalarType.BYTES:
      return reader.bytes();
    case ScalarType.FIXED32:
      return reader.fixed32();
    case ScalarType.SFIXED32:
      return reader.sfixed32();
    case ScalarType.SFIXED64:
      return reader.sfixed64();
    case ScalarType.SINT64:
      return reader.sint64();
    case ScalarType.UINT32:
      return reader.uint32();
    case ScalarType.SINT32:
      return reader.sint32();
  }
}
function writeField(field, value, writer, options) {
  assert(value !== undefined);
  const repeated = field.repeated;
  switch (field.kind) {
    case "scalar":
    case "enum":
      let scalarType = field.kind == "enum" ? ScalarType.INT32 : field.T;
      if (repeated) {
        assert(Array.isArray(value));
        if (field.packed) {
          writePacked(writer, scalarType, field.no, value);
        } else {
          for (const item of value) {
            writeScalar(writer, scalarType, field.no, item);
          }
        }
      } else {
        writeScalar(writer, scalarType, field.no, value);
      }
      break;
    case "message":
      if (repeated) {
        assert(Array.isArray(value));
        for (const item of value) {
          writeMessageField(writer, options, field, item);
        }
      } else {
        writeMessageField(writer, options, field, value);
      }
      break;
    case "map":
      assert(typeof value == "object" && value != null);
      for (const _ref of Object.entries(value)) {
        var _ref2 = _slicedToArray(_ref, 2);
        const key = _ref2[0];
        const val = _ref2[1];
        writeMapEntry(writer, options, field, key, val);
      }
      break;
  }
}
function writeMapEntry(writer, options, field, key, value) {
  writer.tag(field.no, WireType.LengthDelimited);
  writer.fork();
  // javascript only allows number or string for object properties
  // we convert from our representation to the protobuf type
  let keyValue = key;
  // eslint-disable-next-line @typescript-eslint/switch-exhaustiveness-check -- we deliberately handle just the special cases for map keys
  switch (field.K) {
    case ScalarType.INT32:
    case ScalarType.FIXED32:
    case ScalarType.UINT32:
    case ScalarType.SFIXED32:
    case ScalarType.SINT32:
      keyValue = Number.parseInt(key);
      break;
    case ScalarType.BOOL:
      assert(key == "true" || key == "false");
      keyValue = key == "true";
      break;
  }
  // write key, expecting key field number = 1
  writeScalar(writer, field.K, 1, keyValue);
  // write value, expecting value field number = 2
  switch (field.V.kind) {
    case "scalar":
      writeScalar(writer, field.V.T, 2, value);
      break;
    case "enum":
      writeScalar(writer, ScalarType.INT32, 2, value);
      break;
    case "message":
      assert(value !== undefined);
      writer.tag(2, WireType.LengthDelimited).bytes(value.toBinary(options));
      break;
  }
  writer.join();
}
// Value must not be undefined
function writeMessageField(writer, options, field, value) {
  const message = wrapField(field.T, value);
  // eslint-disable-next-line @typescript-eslint/strict-boolean-expressions
  if (field.delimited) writer.tag(field.no, WireType.StartGroup).raw(message.toBinary(options)).tag(field.no, WireType.EndGroup);else writer.tag(field.no, WireType.LengthDelimited).bytes(message.toBinary(options));
}
export { canEmitFieldDefaultValue, debugJsonValue, jsonReadDefaults, jsonWriteDefaults, makeBinaryFormat, makeJsonFormat, makeReadOptions, makeReadOptions$1, makeWriteOptions, makeWriteOptions$1, readDefaults, readEnum, readField, readField$1, readMapEntry, readMapKey, readMessageField, readScalar, readScalar$1, readScalarLTString, tokenIgnoredUnknownEnum, tokenNull, unknownFieldsSymbol, writeDefaults, writeEnum, writeField, writeField$1, writeMapEntry, writeMessageField, writeScalar$1 };
