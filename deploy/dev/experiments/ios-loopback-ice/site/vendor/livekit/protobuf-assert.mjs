// 由 livekit-client.esm.mjs（livekit-client@2.23.0）第 27-162 行按职责切出，行为不变。
// Copyright 2021-2024 Buf Technologies, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
/**
 * Assert that condition is truthy or throw error (with message)
 */
function assert(condition, msg) {
  // eslint-disable-next-line @typescript-eslint/strict-boolean-expressions -- we want the implicit conversion to boolean
  if (!condition) {
    throw new Error(msg);
  }
}
const FLOAT32_MAX = 3.4028234663852886e38,
  FLOAT32_MIN = -34028234663852886e22,
  UINT32_MAX = 0xffffffff,
  INT32_MAX = 0x7fffffff,
  INT32_MIN = -2147483648;
/**
 * Assert a valid signed protobuf 32-bit integer.
 */
function assertInt32(arg) {
  if (typeof arg !== "number") throw new Error("invalid int 32: " + typeof arg);
  if (!Number.isInteger(arg) || arg > INT32_MAX || arg < INT32_MIN) throw new Error("invalid int 32: " + arg); // eslint-disable-line @typescript-eslint/restrict-plus-operands -- we want the implicit conversion to string
}
/**
 * Assert a valid unsigned protobuf 32-bit integer.
 */
function assertUInt32(arg) {
  if (typeof arg !== "number") throw new Error("invalid uint 32: " + typeof arg);
  if (!Number.isInteger(arg) || arg > UINT32_MAX || arg < 0) throw new Error("invalid uint 32: " + arg); // eslint-disable-line @typescript-eslint/restrict-plus-operands -- we want the implicit conversion to string
}
/**
 * Assert a valid protobuf float value.
 */
function assertFloat32(arg) {
  if (typeof arg !== "number") throw new Error("invalid float 32: " + typeof arg);
  if (!Number.isFinite(arg)) return;
  if (arg > FLOAT32_MAX || arg < FLOAT32_MIN) throw new Error("invalid float 32: " + arg); // eslint-disable-line @typescript-eslint/restrict-plus-operands -- we want the implicit conversion to string
}// Copyright 2021-2024 Buf Technologies, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
const enumTypeSymbol = Symbol("@bufbuild/protobuf/enum-type");
/**
 * Get reflection information from a generated enum.
 * If this function is called on something other than a generated
 * enum, it raises an error.
 */
function getEnumType(enumObject) {
  // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access,@typescript-eslint/no-unsafe-assignment,@typescript-eslint/no-explicit-any
  const t = enumObject[enumTypeSymbol];
  assert(t, "missing enum type on enum object");
  return t; // eslint-disable-line @typescript-eslint/no-unsafe-return
}
/**
 * Sets reflection information on a generated enum.
 */
function setEnumType(enumObject, typeName, values, opt) {
  // eslint-disable-next-line @typescript-eslint/no-unsafe-member-access, @typescript-eslint/no-explicit-any
  enumObject[enumTypeSymbol] = makeEnumType(typeName, values.map(v => ({
    no: v.no,
    name: v.name,
    localName: enumObject[v.no]
  })));
}
/**
 * Create a new EnumType with the given values.
 */
function makeEnumType(typeName, values,
// eslint-disable-next-line @typescript-eslint/no-unused-vars
_opt) {
  const names = Object.create(null);
  const numbers = Object.create(null);
  const normalValues = [];
  for (const value of values) {
    // We do not surface options at this time
    // const value: EnumValueInfo = {...v, options: v.options ?? emptyReadonlyObject};
    const n = normalizeEnumValue(value);
    normalValues.push(n);
    names[value.name] = n;
    numbers[value.no] = n;
  }
  return {
    typeName,
    values: normalValues,
    // We do not surface options at this time
    // options: opt?.options ?? Object.create(null),
    findName(name) {
      return names[name];
    },
    findNumber(no) {
      return numbers[no];
    }
  };
}
/**
 * Create a new enum object with the given values.
 * Sets reflection information.
 */
function makeEnum(typeName, values, opt) {
  const enumObject = {};
  for (const value of values) {
    const n = normalizeEnumValue(value);
    enumObject[n.localName] = n.no;
    enumObject[n.no] = n.localName;
  }
  setEnumType(enumObject, typeName, values);
  return enumObject;
}
function normalizeEnumValue(value) {
  if ("localName" in value) {
    return value;
  }
  return Object.assign(Object.assign({}, value), {
    localName: value.name
  });
}
export { FLOAT32_MAX, FLOAT32_MIN, INT32_MAX, INT32_MIN, UINT32_MAX, assert, assertFloat32, assertInt32, assertUInt32, enumTypeSymbol, getEnumType, makeEnum, makeEnumType, normalizeEnumValue, setEnumType };
