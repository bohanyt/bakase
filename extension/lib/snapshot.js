/*! Dibuat oleh `npm run build:extension` di web/ dari extension/src/snapshot.js. Jangan diedit langsung.
* Berisi dom-accessibility-api 0.7.1:

MIT License

Copyright (c) 2020 Sebastian Silbermann

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/
var AppSnapshot = (function(exports) {
	Object.defineProperty(exports, Symbol.toStringTag, { value: "Module" });
	// node_modules/dom-accessibility-api/dist/polyfills/array.from.mjs
	/**
	* @source {https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Array/from#Polyfill}
	* but without thisArg (too hard to type, no need to `this`)
	*/
	var toStr = Object.prototype.toString;
	function isCallable(fn) {
		return typeof fn === "function" || toStr.call(fn) === "[object Function]";
	}
	function toInteger(value) {
		var number = Number(value);
		if (isNaN(number)) return 0;
		if (number === 0 || !isFinite(number)) return number;
		return (number > 0 ? 1 : -1) * Math.floor(Math.abs(number));
	}
	var maxSafeInteger = Math.pow(2, 53) - 1;
	function toLength(value) {
		var len = toInteger(value);
		return Math.min(Math.max(len, 0), maxSafeInteger);
	}
	/**
	* Creates an array from an iterable object.
	* @param iterable An iterable object to convert to an array.
	*/
	/**
	* Creates an array from an iterable object.
	* @param iterable An iterable object to convert to an array.
	* @param mapfn A mapping function to call on every element of the array.
	* @param thisArg Value of 'this' used to invoke the mapfn.
	*/
	function arrayFrom(arrayLike, mapFn) {
		var C = Array;
		var items = Object(arrayLike);
		if (arrayLike == null) throw new TypeError("Array.from requires an array-like object - not null or undefined");
		if (typeof mapFn !== "undefined") {
			if (!isCallable(mapFn)) throw new TypeError("Array.from: when provided, the second argument must be a function");
		}
		var len = toLength(items.length);
		var A = isCallable(C) ? Object(new C(len)) : new Array(len);
		var k = 0;
		var kValue;
		while (k < len) {
			kValue = items[k];
			if (mapFn) A[k] = mapFn(kValue, k);
			else A[k] = kValue;
			k += 1;
		}
		A.length = len;
		return A;
	}
	// node_modules/dom-accessibility-api/dist/polyfills/SetLike.mjs
	function _typeof$1(o) {
		"@babel/helpers - typeof";
		return _typeof$1 = "function" == typeof Symbol && "symbol" == typeof Symbol.iterator ? function(o) {
			return typeof o;
		} : function(o) {
			return o && "function" == typeof Symbol && o.constructor === Symbol && o !== Symbol.prototype ? "symbol" : typeof o;
		}, _typeof$1(o);
	}
	function _classCallCheck(instance, Constructor) {
		if (!(instance instanceof Constructor)) throw new TypeError("Cannot call a class as a function");
	}
	function _defineProperties(target, props) {
		for (var i = 0; i < props.length; i++) {
			var descriptor = props[i];
			descriptor.enumerable = descriptor.enumerable || false;
			descriptor.configurable = true;
			if ("value" in descriptor) descriptor.writable = true;
			Object.defineProperty(target, _toPropertyKey$1(descriptor.key), descriptor);
		}
	}
	function _createClass(Constructor, protoProps, staticProps) {
		if (protoProps) _defineProperties(Constructor.prototype, protoProps);
		if (staticProps) _defineProperties(Constructor, staticProps);
		Object.defineProperty(Constructor, "prototype", { writable: false });
		return Constructor;
	}
	function _defineProperty$1(obj, key, value) {
		key = _toPropertyKey$1(key);
		if (key in obj) Object.defineProperty(obj, key, {
			value,
			enumerable: true,
			configurable: true,
			writable: true
		});
		else obj[key] = value;
		return obj;
	}
	function _toPropertyKey$1(t) {
		var i = _toPrimitive$1(t, "string");
		return "symbol" == _typeof$1(i) ? i : i + "";
	}
	function _toPrimitive$1(t, r) {
		if ("object" != _typeof$1(t) || !t) return t;
		var e = t[Symbol.toPrimitive];
		if (void 0 !== e) {
			var i = e.call(t, r || "default");
			if ("object" != _typeof$1(i)) return i;
			throw new TypeError("@@toPrimitive must return a primitive value.");
		}
		return ("string" === r ? String : Number)(t);
	}
	var SetLike_default = typeof Set === "undefined" ? Set : /* @__PURE__ */ function() {
		function SetLike() {
			var items = arguments.length > 0 && arguments[0] !== void 0 ? arguments[0] : [];
			_classCallCheck(this, SetLike);
			_defineProperty$1(this, "items", void 0);
			this.items = items;
		}
		return _createClass(SetLike, [
			{
				key: "add",
				value: function add(value) {
					if (this.has(value) === false) this.items.push(value);
					return this;
				}
			},
			{
				key: "clear",
				value: function clear() {
					this.items = [];
				}
			},
			{
				key: "delete",
				value: function _delete(value) {
					var previousLength = this.items.length;
					this.items = this.items.filter(function(item) {
						return item !== value;
					});
					return previousLength !== this.items.length;
				}
			},
			{
				key: "forEach",
				value: function forEach(callbackfn) {
					var _this = this;
					this.items.forEach(function(item) {
						callbackfn(item, item, _this);
					});
				}
			},
			{
				key: "has",
				value: function has(value) {
					return this.items.indexOf(value) !== -1;
				}
			},
			{
				key: "size",
				get: function get() {
					return this.items.length;
				}
			}
		]);
	}();
	// node_modules/dom-accessibility-api/dist/getRole.mjs
	/**
	* Safe Element.localName for all supported environments
	* @param element
	*/
	function getLocalName(element) {
		var _element$localName;
		return (_element$localName = element.localName) !== null && _element$localName !== void 0 ? _element$localName : element.tagName.toLowerCase();
	}
	var localNameToRoleMappings = {
		article: "article",
		aside: "complementary",
		button: "button",
		datalist: "listbox",
		dd: "definition",
		details: "group",
		dialog: "dialog",
		dt: "term",
		fieldset: "group",
		figure: "figure",
		form: "form",
		footer: "contentinfo",
		h1: "heading",
		h2: "heading",
		h3: "heading",
		h4: "heading",
		h5: "heading",
		h6: "heading",
		header: "banner",
		hr: "separator",
		html: "document",
		legend: "legend",
		li: "listitem",
		math: "math",
		main: "main",
		menu: "list",
		nav: "navigation",
		ol: "list",
		optgroup: "group",
		option: "option",
		output: "status",
		progress: "progressbar",
		section: "region",
		summary: "button",
		table: "table",
		tbody: "rowgroup",
		textarea: "textbox",
		tfoot: "rowgroup",
		td: "cell",
		th: "columnheader",
		thead: "rowgroup",
		tr: "row",
		ul: "list"
	};
	var prohibitedAttributes = {
		caption: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"]),
		code: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"]),
		deletion: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"]),
		emphasis: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"]),
		generic: /* @__PURE__ */ new Set([
			"aria-label",
			"aria-labelledby",
			"aria-roledescription"
		]),
		insertion: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"]),
		none: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"]),
		paragraph: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"]),
		presentation: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"]),
		strong: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"]),
		subscript: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"]),
		superscript: /* @__PURE__ */ new Set(["aria-label", "aria-labelledby"])
	};
	/**
	*
	* @param element
	* @param role The role used for this element. This is specified to control whether you want to use the implicit or explicit role.
	*/
	function hasGlobalAriaAttributes(element, role) {
		return [
			"aria-atomic",
			"aria-busy",
			"aria-controls",
			"aria-current",
			"aria-description",
			"aria-describedby",
			"aria-details",
			"aria-dropeffect",
			"aria-flowto",
			"aria-grabbed",
			"aria-hidden",
			"aria-keyshortcuts",
			"aria-label",
			"aria-labelledby",
			"aria-live",
			"aria-owns",
			"aria-relevant",
			"aria-roledescription"
		].some(function(attributeName) {
			var _prohibitedAttributes;
			return element.hasAttribute(attributeName) && !((_prohibitedAttributes = prohibitedAttributes[role]) !== null && _prohibitedAttributes !== void 0 && _prohibitedAttributes.has(attributeName));
		});
	}
	function ignorePresentationalRole(element, implicitRole) {
		return hasGlobalAriaAttributes(element, implicitRole);
	}
	function getRole(element) {
		var explicitRole = getExplicitRole(element);
		if (explicitRole === null || presentationRoles.indexOf(explicitRole) !== -1) {
			var implicitRole = getImplicitRole(element);
			if (presentationRoles.indexOf(explicitRole || "") === -1 || ignorePresentationalRole(element, implicitRole || "")) return implicitRole;
		}
		return explicitRole;
	}
	function getImplicitRole(element) {
		var mappedByTag = localNameToRoleMappings[getLocalName(element)];
		if (mappedByTag !== void 0) return mappedByTag;
		switch (getLocalName(element)) {
			case "a":
			case "area":
			case "link":
				if (element.hasAttribute("href")) return "link";
				break;
			case "img":
				if (element.getAttribute("alt") === "" && !ignorePresentationalRole(element, "img")) return "presentation";
				return "img";
			case "input":
				var type = element.type;
				switch (type) {
					case "button":
					case "image":
					case "reset":
					case "submit": return "button";
					case "checkbox":
					case "radio": return type;
					case "range": return "slider";
					case "email":
					case "tel":
					case "text":
					case "url":
						if (element.hasAttribute("list")) return "combobox";
						return "textbox";
					case "search":
						if (element.hasAttribute("list")) return "combobox";
						return "searchbox";
					case "number": return "spinbutton";
					default: return null;
				}
			case "select":
				if (element.hasAttribute("multiple") || element.size > 1) return "listbox";
				return "combobox";
		}
		return null;
	}
	function getExplicitRole(element) {
		var role = element.getAttribute("role");
		if (role !== null) {
			var explicitRole = role.trim().split(" ")[0];
			if (explicitRole.length > 0) return explicitRole;
		}
		return null;
	}
	// node_modules/dom-accessibility-api/dist/util.mjs
	var presentationRoles = ["presentation", "none"];
	function isElement(node) {
		return node !== null && node.nodeType === node.ELEMENT_NODE;
	}
	function isHTMLTableCaptionElement(node) {
		return isElement(node) && getLocalName(node) === "caption";
	}
	function isHTMLInputElement(node) {
		return isElement(node) && getLocalName(node) === "input";
	}
	function isHTMLOptGroupElement(node) {
		return isElement(node) && getLocalName(node) === "optgroup";
	}
	function isHTMLSelectElement(node) {
		return isElement(node) && getLocalName(node) === "select";
	}
	function isHTMLTableElement(node) {
		return isElement(node) && getLocalName(node) === "table";
	}
	function isHTMLTextAreaElement(node) {
		return isElement(node) && getLocalName(node) === "textarea";
	}
	function safeWindow(node) {
		var defaultView = (node.ownerDocument === null ? node : node.ownerDocument).defaultView;
		if (defaultView === null) throw new TypeError("no window available");
		return defaultView;
	}
	function isHTMLFieldSetElement(node) {
		return isElement(node) && getLocalName(node) === "fieldset";
	}
	function isHTMLLegendElement(node) {
		return isElement(node) && getLocalName(node) === "legend";
	}
	function isHTMLSlotElement(node) {
		return isElement(node) && getLocalName(node) === "slot";
	}
	function isSVGElement(node) {
		return isElement(node) && node.ownerSVGElement !== void 0;
	}
	function isSVGSVGElement(node) {
		return isElement(node) && getLocalName(node) === "svg";
	}
	function isSVGTitleElement(node) {
		return isSVGElement(node) && getLocalName(node) === "title";
	}
	/**
	*
	* @param {Node} node -
	* @param {string} attributeName -
	* @returns {Element[]} -
	*/
	function queryIdRefs(node, attributeName) {
		if (isElement(node) && node.hasAttribute(attributeName)) {
			var ids = node.getAttribute(attributeName).split(" ");
			var root = node.getRootNode ? node.getRootNode() : node.ownerDocument;
			return ids.map(function(id) {
				return root.getElementById(id);
			}).filter(function(element) {
				return element !== null;
			});
		}
		return [];
	}
	function hasAnyConcreteRoles(node, roles) {
		if (isElement(node)) return roles.indexOf(getRole(node)) !== -1;
		return false;
	}
	// node_modules/dom-accessibility-api/dist/accessible-name-and-description.mjs
	/**
	* implements https://w3c.github.io/accname/
	*/
	/**
	*  A string of characters where all carriage returns, newlines, tabs, and form-feeds are replaced with a single space, and multiple spaces are reduced to a single space. The string contains only character data; it does not contain any markup.
	*/
	/**
	* interface for an options-bag where `window.getComputedStyle` can be mocked
	*/
	/**
	*
	* @param {string} string -
	* @returns {FlatString} -
	*/
	function asFlatString(s) {
		return s.trim().replace(/\s\s+/g, " ");
	}
	/**
	*
	* @param node -
	* @param options - These are not optional to prevent accidentally calling it without options in `computeAccessibleName`
	* @returns {boolean} -
	*/
	function isHidden(node, getComputedStyleImplementation) {
		if (!isElement(node)) return false;
		if (node.hasAttribute("hidden") || node.getAttribute("aria-hidden") === "true") return true;
		var style = getComputedStyleImplementation(node);
		return style.getPropertyValue("display") === "none" || style.getPropertyValue("visibility") === "hidden";
	}
	/**
	* @param {Node} node -
	* @returns {boolean} - As defined in step 2E of https://w3c.github.io/accname/#mapping_additional_nd_te
	*/
	function isControl(node) {
		return hasAnyConcreteRoles(node, [
			"button",
			"combobox",
			"listbox",
			"textbox"
		]) || hasAbstractRole(node, "range");
	}
	function hasAbstractRole(node, role) {
		if (!isElement(node)) return false;
		switch (role) {
			case "range": return hasAnyConcreteRoles(node, [
				"meter",
				"progressbar",
				"scrollbar",
				"slider",
				"spinbutton"
			]);
			default: throw new TypeError("No knowledge about abstract role '".concat(role, "'. This is likely a bug :("));
		}
	}
	/**
	* element.querySelectorAll but also considers owned tree
	* @param element
	* @param selectors
	*/
	function querySelectorAllSubtree(element, selectors) {
		var elements = arrayFrom(element.querySelectorAll(selectors));
		queryIdRefs(element, "aria-owns").forEach(function(root) {
			elements.push.apply(elements, arrayFrom(root.querySelectorAll(selectors)));
		});
		return elements;
	}
	function querySelectedOptions(listbox) {
		if (isHTMLSelectElement(listbox)) return listbox.selectedOptions || querySelectorAllSubtree(listbox, "[selected]");
		return querySelectorAllSubtree(listbox, "[aria-selected=\"true\"]");
	}
	function isMarkedPresentational(node) {
		return hasAnyConcreteRoles(node, presentationRoles);
	}
	/**
	* Elements specifically listed in html-aam
	*
	* We don't need this for `label` or `legend` elements.
	* Their implicit roles already allow "naming from content".
	*
	* sources:
	*
	* - https://w3c.github.io/html-aam/#table-element
	*/
	function isNativeHostLanguageTextAlternativeElement(node) {
		return isHTMLTableCaptionElement(node);
	}
	/**
	* https://w3c.github.io/aria/#namefromcontent
	*/
	function allowsNameFromContent(node) {
		return hasAnyConcreteRoles(node, [
			"button",
			"cell",
			"checkbox",
			"columnheader",
			"gridcell",
			"heading",
			"label",
			"legend",
			"link",
			"menuitem",
			"menuitemcheckbox",
			"menuitemradio",
			"option",
			"radio",
			"row",
			"rowheader",
			"switch",
			"tab",
			"tooltip",
			"treeitem"
		]);
	}
	function isDescendantOfNativeHostLanguageTextAlternativeElement(node) {
		return false;
	}
	function getValueOfTextbox(element) {
		if (isHTMLInputElement(element) || isHTMLTextAreaElement(element)) return element.value;
		return element.textContent || "";
	}
	function getTextualContent(declaration) {
		var content = declaration.getPropertyValue("content");
		if (/^["'].*["']$/.test(content)) return content.slice(1, -1);
		return "";
	}
	/**
	* https://html.spec.whatwg.org/multipage/forms.html#category-label
	* TODO: form-associated custom elements
	* @param element
	*/
	function isLabelableElement(element) {
		var localName = getLocalName(element);
		return localName === "button" || localName === "input" && element.getAttribute("type") !== "hidden" || localName === "meter" || localName === "output" || localName === "progress" || localName === "select" || localName === "textarea";
	}
	/**
	* > [...], then the first such descendant in tree order is the label element's labeled control.
	* -- https://html.spec.whatwg.org/multipage/forms.html#labeled-control
	* @param element
	*/
	function findLabelableElement(element) {
		if (isLabelableElement(element)) return element;
		var labelableElement = null;
		element.childNodes.forEach(function(childNode) {
			if (labelableElement === null && isElement(childNode)) {
				var descendantLabelableElement = findLabelableElement(childNode);
				if (descendantLabelableElement !== null) labelableElement = descendantLabelableElement;
			}
		});
		return labelableElement;
	}
	/**
	* Polyfill of HTMLLabelElement.control
	* https://html.spec.whatwg.org/multipage/forms.html#labeled-control
	* @param label
	*/
	function getControlOfLabel(label) {
		if (label.control !== void 0) return label.control;
		var htmlFor = label.getAttribute("for");
		if (htmlFor !== null) return label.ownerDocument.getElementById(htmlFor);
		return findLabelableElement(label);
	}
	/**
	* Polyfill of HTMLInputElement.labels
	* https://developer.mozilla.org/en-US/docs/Web/API/HTMLInputElement/labels
	* @param element
	*/
	function getLabels(element) {
		var labelsProperty = element.labels;
		if (labelsProperty === null) return labelsProperty;
		if (labelsProperty !== void 0) return arrayFrom(labelsProperty);
		if (!isLabelableElement(element)) return null;
		var document = element.ownerDocument;
		return arrayFrom(document.querySelectorAll("label")).filter(function(label) {
			return getControlOfLabel(label) === element;
		});
	}
	/**
	* Gets the contents of a slot used for computing the accname
	* @param slot
	*/
	function getSlotContents(slot) {
		var assignedNodes = slot.assignedNodes();
		if (assignedNodes.length === 0) return arrayFrom(slot.childNodes);
		return assignedNodes;
	}
	/**
	* implements https://w3c.github.io/accname/#mapping_additional_nd_te
	* @param root
	* @param options
	* @returns
	*/
	function computeTextAlternative(root) {
		var options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
		var consultedNodes = new SetLike_default();
		var computedStyles = typeof Map === "undefined" ? void 0 : /* @__PURE__ */ new Map();
		var window = safeWindow(root);
		var _options$compute = options.compute, compute = _options$compute === void 0 ? "name" : _options$compute, _options$computedStyl = options.computedStyleSupportsPseudoElements, computedStyleSupportsPseudoElements = _options$computedStyl === void 0 ? options.getComputedStyle !== void 0 : _options$computedStyl, _options$getComputedS = options.getComputedStyle, uncachedGetComputedStyle = _options$getComputedS === void 0 ? window.getComputedStyle.bind(window) : _options$getComputedS, _options$hidden = options.hidden, hidden = _options$hidden === void 0 ? false : _options$hidden;
		var getComputedStyle = function getComputedStyle(el, pseudoElement) {
			if (pseudoElement !== void 0) throw new Error("use uncachedGetComputedStyle directly for pseudo elements");
			if (computedStyles === void 0) return uncachedGetComputedStyle(el);
			var cachedStyles = computedStyles.get(el);
			if (cachedStyles) return cachedStyles;
			var style = uncachedGetComputedStyle(el, pseudoElement);
			computedStyles.set(el, style);
			return style;
		};
		function computeMiscTextAlternative(node, context) {
			var accumulatedText = "";
			if (isElement(node) && computedStyleSupportsPseudoElements) {
				var beforeContent = getTextualContent(uncachedGetComputedStyle(node, "::before"));
				accumulatedText = "".concat(beforeContent, " ").concat(accumulatedText);
			}
			(isHTMLSlotElement(node) ? getSlotContents(node) : arrayFrom(node.childNodes).concat(queryIdRefs(node, "aria-owns"))).forEach(function(child) {
				var result = computeTextAlternative(child, {
					isEmbeddedInLabel: context.isEmbeddedInLabel,
					isReferenced: false,
					recursion: true
				});
				var separator = (isElement(child) ? getComputedStyle(child).getPropertyValue("display") : "inline") !== "inline" ? " " : "";
				accumulatedText += "".concat(separator).concat(result).concat(separator);
			});
			if (isElement(node) && computedStyleSupportsPseudoElements) {
				var afterContent = getTextualContent(uncachedGetComputedStyle(node, "::after"));
				accumulatedText = "".concat(accumulatedText, " ").concat(afterContent);
			}
			return accumulatedText.trim();
		}
		/**
		*
		* @param element
		* @param attributeName
		* @returns A string non-empty string or `null`
		*/
		function useAttribute(element, attributeName) {
			var attribute = element.getAttributeNode(attributeName);
			if (attribute !== null && !consultedNodes.has(attribute) && attribute.value.trim() !== "") {
				consultedNodes.add(attribute);
				return attribute.value;
			}
			return null;
		}
		function computeTooltipAttributeValue(node) {
			if (!isElement(node)) return null;
			return useAttribute(node, "title");
		}
		function computeElementTextAlternative(node) {
			if (!isElement(node)) return null;
			if (isHTMLFieldSetElement(node)) {
				consultedNodes.add(node);
				var children = arrayFrom(node.childNodes);
				for (var i = 0; i < children.length; i += 1) {
					var child = children[i];
					if (isHTMLLegendElement(child)) return computeTextAlternative(child, {
						isEmbeddedInLabel: false,
						isReferenced: false,
						recursion: false
					});
				}
			} else if (isHTMLTableElement(node)) {
				consultedNodes.add(node);
				var _children = arrayFrom(node.childNodes);
				for (var _i = 0; _i < _children.length; _i += 1) {
					var _child = _children[_i];
					if (isHTMLTableCaptionElement(_child)) return computeTextAlternative(_child, {
						isEmbeddedInLabel: false,
						isReferenced: false,
						recursion: false
					});
				}
			} else if (isSVGSVGElement(node)) {
				consultedNodes.add(node);
				var _children2 = arrayFrom(node.childNodes);
				for (var _i2 = 0; _i2 < _children2.length; _i2 += 1) {
					var _child2 = _children2[_i2];
					if (isSVGTitleElement(_child2)) return _child2.textContent;
				}
				return null;
			} else if (getLocalName(node) === "img" || getLocalName(node) === "area") {
				var nameFromAlt = useAttribute(node, "alt");
				if (nameFromAlt !== null) return nameFromAlt;
			} else if (isHTMLOptGroupElement(node)) {
				var nameFromLabel = useAttribute(node, "label");
				if (nameFromLabel !== null) return nameFromLabel;
			}
			if (isHTMLInputElement(node) && (node.type === "button" || node.type === "submit" || node.type === "reset")) {
				var nameFromValue = useAttribute(node, "value");
				if (nameFromValue !== null) return nameFromValue;
				if (node.type === "submit") return "Submit";
				if (node.type === "reset") return "Reset";
			}
			var labels = getLabels(node);
			if (labels !== null && labels.length !== 0) {
				consultedNodes.add(node);
				return arrayFrom(labels).map(function(element) {
					return computeTextAlternative(element, {
						isEmbeddedInLabel: true,
						isReferenced: false,
						recursion: true
					});
				}).filter(function(label) {
					return label.length > 0;
				}).join(" ");
			}
			if (isHTMLInputElement(node) && node.type === "image") {
				var _nameFromAlt = useAttribute(node, "alt");
				if (_nameFromAlt !== null) return _nameFromAlt;
				var nameFromTitle = useAttribute(node, "title");
				if (nameFromTitle !== null) return nameFromTitle;
				return "Submit Query";
			}
			if (hasAnyConcreteRoles(node, ["button"])) {
				var nameFromSubTree = computeMiscTextAlternative(node, {
					isEmbeddedInLabel: false,
					isReferenced: false
				});
				if (nameFromSubTree !== "") return nameFromSubTree;
			}
			return null;
		}
		function computeTextAlternative(current, context) {
			if (consultedNodes.has(current)) return "";
			if (!hidden && isHidden(current, getComputedStyle) && !context.isReferenced) {
				consultedNodes.add(current);
				return "";
			}
			var labelAttributeNode = isElement(current) ? current.getAttributeNode("aria-labelledby") : null;
			var labelElements = labelAttributeNode !== null && !consultedNodes.has(labelAttributeNode) ? queryIdRefs(current, "aria-labelledby") : [];
			if (compute === "name" && !context.isReferenced && labelElements.length > 0) {
				consultedNodes.add(labelAttributeNode);
				return labelElements.map(function(element) {
					return computeTextAlternative(element, {
						isEmbeddedInLabel: context.isEmbeddedInLabel,
						isReferenced: true,
						recursion: false
					});
				}).join(" ");
			}
			var skipToStep2E = context.recursion && isControl(current) && compute === "name";
			if (!skipToStep2E) {
				var ariaLabel = (isElement(current) && current.getAttribute("aria-label") || "").trim();
				if (ariaLabel !== "" && compute === "name") {
					consultedNodes.add(current);
					return ariaLabel;
				}
				if (!isMarkedPresentational(current)) {
					var elementTextAlternative = computeElementTextAlternative(current);
					if (elementTextAlternative !== null) {
						consultedNodes.add(current);
						return elementTextAlternative;
					}
				}
			}
			if (hasAnyConcreteRoles(current, ["menu"])) {
				consultedNodes.add(current);
				return "";
			}
			if (skipToStep2E || context.isEmbeddedInLabel || context.isReferenced) {
				if (hasAnyConcreteRoles(current, ["combobox", "listbox"])) {
					consultedNodes.add(current);
					var selectedOptions = querySelectedOptions(current);
					if (selectedOptions.length === 0) return isHTMLInputElement(current) ? current.value : "";
					return arrayFrom(selectedOptions).map(function(selectedOption) {
						return computeTextAlternative(selectedOption, {
							isEmbeddedInLabel: context.isEmbeddedInLabel,
							isReferenced: false,
							recursion: true
						});
					}).join(" ");
				}
				if (hasAbstractRole(current, "range")) {
					consultedNodes.add(current);
					if (current.hasAttribute("aria-valuetext")) return current.getAttribute("aria-valuetext");
					if (current.hasAttribute("aria-valuenow")) return current.getAttribute("aria-valuenow");
					return current.getAttribute("value") || "";
				}
				if (hasAnyConcreteRoles(current, ["textbox"])) {
					consultedNodes.add(current);
					return getValueOfTextbox(current);
				}
			}
			if (allowsNameFromContent(current) || isElement(current) && context.isReferenced || isNativeHostLanguageTextAlternativeElement(current) || isDescendantOfNativeHostLanguageTextAlternativeElement(current)) {
				var accumulatedText2F = computeMiscTextAlternative(current, {
					isEmbeddedInLabel: context.isEmbeddedInLabel,
					isReferenced: false
				});
				if (accumulatedText2F !== "") {
					consultedNodes.add(current);
					return accumulatedText2F;
				}
			}
			if (current.nodeType === current.TEXT_NODE) {
				consultedNodes.add(current);
				return current.textContent || "";
			}
			if (context.recursion) {
				consultedNodes.add(current);
				return computeMiscTextAlternative(current, {
					isEmbeddedInLabel: context.isEmbeddedInLabel,
					isReferenced: false
				});
			}
			var tooltipAttributeValue = computeTooltipAttributeValue(current);
			if (tooltipAttributeValue !== null) {
				consultedNodes.add(current);
				return tooltipAttributeValue;
			}
			consultedNodes.add(current);
			return "";
		}
		return asFlatString(computeTextAlternative(root, {
			isEmbeddedInLabel: false,
			isReferenced: compute === "description",
			recursion: false
		}));
	}
	// node_modules/dom-accessibility-api/dist/accessible-description.mjs
	function _typeof(o) {
		"@babel/helpers - typeof";
		return _typeof = "function" == typeof Symbol && "symbol" == typeof Symbol.iterator ? function(o) {
			return typeof o;
		} : function(o) {
			return o && "function" == typeof Symbol && o.constructor === Symbol && o !== Symbol.prototype ? "symbol" : typeof o;
		}, _typeof(o);
	}
	function ownKeys(e, r) {
		var t = Object.keys(e);
		if (Object.getOwnPropertySymbols) {
			var o = Object.getOwnPropertySymbols(e);
			r && (o = o.filter(function(r) {
				return Object.getOwnPropertyDescriptor(e, r).enumerable;
			})), t.push.apply(t, o);
		}
		return t;
	}
	function _objectSpread(e) {
		for (var r = 1; r < arguments.length; r++) {
			var t = null != arguments[r] ? arguments[r] : {};
			r % 2 ? ownKeys(Object(t), !0).forEach(function(r) {
				_defineProperty(e, r, t[r]);
			}) : Object.getOwnPropertyDescriptors ? Object.defineProperties(e, Object.getOwnPropertyDescriptors(t)) : ownKeys(Object(t)).forEach(function(r) {
				Object.defineProperty(e, r, Object.getOwnPropertyDescriptor(t, r));
			});
		}
		return e;
	}
	function _defineProperty(obj, key, value) {
		key = _toPropertyKey(key);
		if (key in obj) Object.defineProperty(obj, key, {
			value,
			enumerable: true,
			configurable: true,
			writable: true
		});
		else obj[key] = value;
		return obj;
	}
	function _toPropertyKey(t) {
		var i = _toPrimitive(t, "string");
		return "symbol" == _typeof(i) ? i : i + "";
	}
	function _toPrimitive(t, r) {
		if ("object" != _typeof(t) || !t) return t;
		var e = t[Symbol.toPrimitive];
		if (void 0 !== e) {
			var i = e.call(t, r || "default");
			if ("object" != _typeof(i)) return i;
			throw new TypeError("@@toPrimitive must return a primitive value.");
		}
		return ("string" === r ? String : Number)(t);
	}
	/**
	* @param root
	* @param options
	* @returns
	*/
	function computeAccessibleDescription(root) {
		var options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
		var description = queryIdRefs(root, "aria-describedby").map(function(element) {
			return computeTextAlternative(element, _objectSpread(_objectSpread({}, options), {}, { compute: "description" }));
		}).join(" ");
		if (description === "") {
			var ariaDescription = root.getAttribute("aria-description");
			description = ariaDescription === null ? "" : ariaDescription;
		}
		if (description === "") {
			var title = root.getAttribute("title");
			description = title === null ? "" : title;
		}
		return description;
	}
	// node_modules/dom-accessibility-api/dist/accessible-name.mjs
	/**
	* https://w3c.github.io/aria/#namefromprohibited
	*/
	function prohibitsNaming(node) {
		return hasAnyConcreteRoles(node, [
			"caption",
			"code",
			"deletion",
			"emphasis",
			"generic",
			"insertion",
			"none",
			"paragraph",
			"presentation",
			"strong",
			"subscript",
			"superscript"
		]);
	}
	/**
	* implements https://w3c.github.io/accname/#mapping_additional_nd_name
	* @param root
	* @param options
	* @returns
	*/
	function computeAccessibleName(root) {
		var options = arguments.length > 1 && arguments[1] !== void 0 ? arguments[1] : {};
		if (prohibitsNaming(root)) return "";
		return computeTextAlternative(root, options);
	}
	// node_modules/dom-accessibility-api/dist/is-disabled.mjs
	var elementsSupportingDisabledAttribute = /* @__PURE__ */ new Set([
		"button",
		"fieldset",
		"input",
		"optgroup",
		"option",
		"select",
		"textarea"
	]);
	/**
	* Check if an element is disabled
	* https://www.w3.org/TR/html-aam-1.0/#html-attribute-state-and-property-mappings
	* https://www.w3.org/TR/wai-aria-1.1/#aria-disabled
	*
	* @param element
	* @returns {boolean} true if disabled, otherwise false
	*/
	function isDisabled(element) {
		var localName = getLocalName(element);
		return elementsSupportingDisabledAttribute.has(localName) && element.hasAttribute("disabled") ? true : element.getAttribute("aria-disabled") === "true";
	}
	// ../extension/src/snapshot.js
	var NOTE = "\n[Terpotong: halaman lebih panjang dari batas baca.]";
	var MAX_ROWS = 200;
	var MAX_NAME = 200;
	var MAX_TEXT = 2e3;
	var FIELDS = /* @__PURE__ */ new Set([
		"textbox",
		"searchbox",
		"combobox",
		"spinbutton",
		"slider",
		"checkbox",
		"radio",
		"switch",
		"listbox"
	]);
	var ACTIONS = /* @__PURE__ */ new Set([
		"button",
		"link",
		"menuitem",
		"menuitemcheckbox",
		"menuitemradio",
		"tab",
		"option",
		"treeitem"
	]);
	var TABLES = /* @__PURE__ */ new Set([
		"table",
		"grid",
		"treegrid"
	]);
	var TABLE_SELECTOR = "table, [role=\"table\"], [role=\"grid\"], [role=\"treegrid\"]";
	var ROW_SELECTOR = "[role=\"row\"], tr";
	var MESSAGES = /* @__PURE__ */ new Set(["alert", "status"]);
	var ALWAYS = /* @__PURE__ */ new Set([
		"dialog",
		"alertdialog",
		"tablist",
		"toolbar",
		"menu",
		"menubar"
	]);
	var IF_NAMED = /* @__PURE__ */ new Set([
		"navigation",
		"form",
		"search",
		"region",
		"group",
		"radiogroup",
		"tabpanel",
		"tree",
		"complementary"
	]);
	var CHECKABLE = /* @__PURE__ */ new Set([
		"checkbox",
		"radio",
		"switch",
		"menuitemcheckbox",
		"menuitemradio"
	]);
	var NAME_FROM_CONTENT = /* @__PURE__ */ new Set([
		...ACTIONS,
		...CHECKABLE,
		"heading",
		"cell",
		"gridcell",
		"columnheader",
		"rowheader",
		"row",
		"tooltip"
	]);
	var SKIP_TAGS = /* @__PURE__ */ new Set([
		"SCRIPT",
		"STYLE",
		"NOSCRIPT",
		"TEMPLATE",
		"HEAD",
		"META",
		"LINK",
		"OBJECT",
		"EMBED"
	]);
	var CONTROL_TAGS = /* @__PURE__ */ new Set([
		"INPUT",
		"SELECT",
		"TEXTAREA"
	]);
	var NOT_FIELD_INPUTS = /* @__PURE__ */ new Set([
		"hidden",
		"submit",
		"reset",
		"button",
		"image"
	]);
	var EDITABLE = "[contenteditable]:not([contenteditable=\"false\"])";
	var FIELD_SELECTOR = `input, select, textarea, ${EDITABLE}, [role="textbox"], [role="searchbox"], [role="combobox"], [role="spinbutton"], [role="slider"], [role="listbox"]`;
	var MEANINGFUL = `a[href], button, input, select, textarea, [role], h1, h2, h3, h4, h5, h6, table, img[alt], details, summary, iframe, ${EDITABLE}, [aria-expanded]`;
	var SECRET = /(^|[^a-z])(pin|otp|cvv|cvc|passcode|password|sandi)([^a-z]|$)/i;
	var SECRET_TYPES = /* @__PURE__ */ new Set([
		"hidden",
		"password",
		"file"
	]);
	var clean = (s, max = MAX_NAME) => {
		const t = (s || "").replace(/\s+/g, " ").trim();
		return t.length > max ? `${t.slice(0, max)}…` : t;
	};
	var quote = (s) => `"${s.replace(/"/g, "\\\"")}"`;
	/** Induk yang benar-benar merender elemen: slot tempat ia diproyeksikan, induk biasa, atau host
	* shadow root. */
	var parentOf = (el) => el.assignedSlot || el.parentElement || el.getRootNode()?.host || null;
	function scrollBox(el) {
		return el.clientHeight > 0 && el.scrollHeight > el.clientHeight && /^(auto|scroll|overlay)$/.test(getComputedStyle(el).overflowY);
	}
	function scrollerOf(el) {
		if (el?.matches(TABLE_SELECTOR) && !scrollBox(el)) {
			for (const row of el.querySelectorAll(ROW_SELECTOR)) for (let parent = parentOf(row); parent && parent !== el; parent = parentOf(parent)) if (scrollBox(parent)) return parent;
		}
		for (let parent = el; parent; parent = parentOf(parent)) if (scrollBox(parent)) return parent;
		return document.scrollingElement;
	}
	function reveal(el) {
		el.scrollIntoView({
			block: "nearest",
			inline: "nearest",
			behavior: "instant"
		});
	}
	async function act({ id, pageId, action, ref, direction, url, remaining_ms }) {
		const receivedAt = performance.now();
		if (!Number.isFinite(remaining_ms) || remaining_ms <= 0 || remaining_ms > 8e3) return { status: "timeout" };
		const localDeadline = receivedAt + remaining_ms;
		const check = (type) => chrome.runtime.sendMessage({
			type,
			id,
			pageId
		}).catch(() => false);
		if (!await check("app:izin-aksi") || !await check("app:cek-aksi") || performance.now() >= localDeadline || location.href !== url || document.visibilityState !== "visible") return { status: "unavailable" };
		const entry = ref && globalThis.__appRefs?.get(ref);
		const el = entry?.node.deref();
		if (ref && (!el?.isConnected || !el.getClientRects().length || hiddenUp(el) || el.closest(ROW_SELECTOR)?.getAttribute("aria-rowindex") !== entry.rowIndex || getComputedStyle(el).visibility !== "visible")) return { status: "stale_ref" };
		if (action === "sorot") {
			reveal(el);
			globalThis.__appOutline?.cancel();
			const marker = {
				outline: "3px solid #0078d4",
				outlineOffset: "-3px"
			};
			const outline = el.animate([marker, marker], {
				duration: 2500,
				fill: "none"
			});
			globalThis.__appOutline = outline;
			outline.onfinish = () => {
				outline.cancel();
				if (globalThis.__appOutline === outline) globalThis.__appOutline = null;
			};
			return { status: "highlighted" };
		}
		const scroller = scrollerOf(el);
		if (!scroller) return { status: "failed" };
		const top = scroller.scrollTop;
		const left = scroller.scrollLeft;
		const pageTop = scrollY;
		const pageLeft = scrollX;
		const scope = el?.closest(TABLE_SELECTOR) || scroller;
		globalThis.__appScope = new WeakRef(scope);
		const viewport = scroller === document.scrollingElement ? innerHeight : scroller.clientHeight;
		if (direction === "to") reveal(el);
		else scroller.scrollBy({
			top: Math.max(1, Math.floor(viewport * .85)) * (direction === "up" ? -1 : 1),
			behavior: "instant"
		});
		return {
			status: "scrolled",
			moved: scroller.scrollTop !== top || scroller.scrollLeft !== left || scrollY !== pageTop || scrollX !== pageLeft
		};
	}
	function watchFrame(pageId, url) {
		globalThis.__appFrameWatch?.stop();
		const changes = globalThis.navigation;
		function stop() {
			changes?.removeEventListener("currententrychange", changed);
			window.removeEventListener("pagehide", gone);
			globalThis.__appFrameWatch = null;
		}
		function gone(event) {
			if (!event.isTrusted) return;
			stop();
			chrome.runtime.sendMessage({
				type: "app:frame-berubah",
				pageId
			}).catch(() => {});
		}
		function changed(event) {
			if (location.href !== url) gone(event);
		}
		globalThis.__appFrameWatch = {
			pageId,
			stop
		};
		changes?.addEventListener("currententrychange", changed);
		window.addEventListener("pagehide", gone);
	}
	function unwatchFrame(pageId) {
		if (globalThis.__appFrameWatch?.pageId === pageId) globalThis.__appFrameWatch.stop();
	}
	/** Elemen yang dirender. `display: contents` tidak punya kotak sendiri tapi isinya tetap tampil,
	* kecuali isinya disembunyikan dengan content-visibility; isi <details> yang tertutup tidak punya
	* kotak dan tidak tampil. */
	function rendered(el) {
		const s = getComputedStyle(el);
		if (s.display === "none") return false;
		if (s.display === "contents") return s.contentVisibility !== "hidden";
		return el.checkVisibility ? el.checkVisibility() : true;
	}
	/** Ada induk (juga di luar shadow root) yang aria-hidden atau inert. */
	function hiddenUp(el) {
		for (let e = el; e; e = parentOf(e)) if (e.getAttribute("aria-hidden") === "true" || e.inert) return true;
		return false;
	}
	/** Anak yang benar-benar dirender: isi shadow root, dan isi slot yang diproyeksikan. */
	function childrenOf(node) {
		if (node.shadowRoot) return node.shadowRoot.childNodes;
		if (node.tagName === "SLOT") {
			const assigned = node.assignedNodes({ flatten: true });
			return assigned.length ? assigned : node.childNodes;
		}
		return node.childNodes;
	}
	var editableRoot = (el) => el.isContentEditable && !parentOf(el)?.isContentEditable;
	/** Peran elemen. Input tanpa peran ARIA (password, tanggal, dan sejenisnya) dan area teks yang
	* bisa diedit tetap sebuah isian, juga kalau diberi peran lain (misalnya heading). */
	function roleOf(el) {
		const role = getRole(el);
		if (editableRoot(el) && !FIELDS.has(role)) return "textbox";
		if (role) return role;
		if (el.tagName === "INPUT" && !NOT_FIELD_INPUTS.has((el.type || "").toLowerCase())) return "textbox";
		return null;
	}
	/** Elemen yang dirujuk lewat atribut id (aria-labelledby, aria-describedby). */
	function referenced(el, attr) {
		const root = el.getRootNode();
		return (el.getAttribute(attr) || "").split(/\s+/).filter(Boolean).map((id) => root.getElementById?.(id) ?? document.getElementById(id)).filter(Boolean);
	}
	/** Petunjuk nama isian untuk deteksi rahasia, tanpa menghitung nama accessible (yang bisa
	* memuat nilai isian lain). */
	/** Teks label beserta alt, aria-label, dan title di dalamnya (label bisa berupa gambar "PIN"). */
	function labelHint(node, labelHints) {
		if (!labelHints.has(node)) {
			const parts = [
				node.textContent,
				node.getAttribute?.("alt"),
				node.getAttribute?.("aria-label"),
				node.getAttribute?.("title")
			];
			for (const e of node.querySelectorAll?.("[alt], [aria-label], [title]") ?? []) parts.push(e.getAttribute("alt"), e.getAttribute("aria-label"), e.getAttribute("title"));
			labelHints.set(node, parts.filter(Boolean).join(" "));
		}
		return labelHints.get(node);
	}
	function hintOf(el, labelHints) {
		const parts = [
			el.getAttribute("aria-label"),
			el.getAttribute("name"),
			el.id,
			el.getAttribute("placeholder"),
			el.getAttribute("autocomplete"),
			el.getAttribute("title")
		];
		for (const r of referenced(el, "aria-labelledby")) parts.push(labelHint(r, labelHints));
		if (el.labels) for (const l of el.labels) parts.push(labelHint(l, labelHints));
		return parts.filter(Boolean).join(" ");
	}
	/** Isian rahasia: password, isian tersembunyi, isian yang disamarkan atau tak terlihat (misalnya
	* input PIN di balik titik-titik), dan isian yang namanya rahasia. Cek yang murah dulu. */
	function isSecret(el, labelHints) {
		if (SECRET_TYPES.has((el.type || "").toLowerCase())) return true;
		if (/one-time-code|password|cc-number|cc-csc/i.test(el.getAttribute("autocomplete") || "")) return true;
		if (SECRET.test(hintOf(el, labelHints))) return true;
		const r = el.getBoundingClientRect();
		if (r.width <= 2 || r.height <= 2) return true;
		const s = getComputedStyle(el);
		if (s.webkitTextSecurity && s.webkitTextSecurity !== "none") return true;
		for (let e = el; e && e !== document.documentElement; e = parentOf(e)) if (Number(getComputedStyle(e).opacity) < .1) return true;
		return false;
	}
	function states(el, role) {
		const s = [];
		const a = (n) => el.getAttribute(n);
		if (el.required || a("aria-required") === "true") s.push("wajib");
		if (a("aria-invalid") && a("aria-invalid") !== "false") s.push("error");
		if (isDisabled(el) || a("aria-disabled") === "true") s.push("nonaktif");
		if (el.readOnly || a("aria-readonly") === "true") s.push("hanya-baca");
		if (a("aria-expanded") === "true") s.push("terbuka");
		if (a("aria-expanded") === "false") s.push("tertutup");
		if (CHECKABLE.has(role)) {
			const c = a("aria-checked") ?? (el.indeterminate ? "mixed" : String(!!el.checked));
			s.push(c === "true" ? "dicentang" : c === "mixed" ? "sebagian" : "tidak dicentang");
		}
		if (a("aria-selected") === "true" || role === "option" && el.selected) s.push("dipilih");
		if (a("aria-pressed") === "true") s.push("ditekan");
		if (a("aria-current") && a("aria-current") !== "false") s.push("saat ini");
		return s;
	}
	function readPage(prefix = "", base = 0, afterScroll = false) {
		const labelHints = /* @__PURE__ */ new WeakMap();
		const refs = /* @__PURE__ */ new Map();
		globalThis.__appRefs = refs;
		const refEnds = [];
		let seq = 0;
		let pendingRef;
		const ref = (el) => {
			seq += 1;
			const id = `${prefix}e${base + seq}`;
			pendingRef = id;
			refs.set(id, {
				node: new WeakRef(el),
				rowIndex: el.closest(ROW_SELECTOR)?.getAttribute("aria-rowindex")
			});
			return `[${id}]`;
		};
		const roots = [document];
		const shadowUp = /* @__PURE__ */ new Set();
		for (let i = 0; i < roots.length; i++) for (const el of roots[i].querySelectorAll("*")) {
			if (!el.shadowRoot) continue;
			roots.push(el.shadowRoot);
			for (let e = el; e && !shadowUp.has(e); e = parentOf(e)) shadowUp.add(e);
		}
		const deepAll = (selector) => {
			const found = [];
			for (const root of roots) for (const el of root.querySelectorAll(selector)) found.push(el);
			return found;
		};
		const secrets = /* @__PURE__ */ new Set();
		const secretUp = /* @__PURE__ */ new Set();
		const checked = /* @__PURE__ */ new Set();
		for (const el of deepAll(FIELD_SELECTOR)) {
			checked.add(el);
			if (CHECKABLE.has(roleOf(el)) || !isSecret(el, labelHints)) continue;
			secrets.add(el);
			for (let e = parentOf(el); e && !secretUp.has(e); e = parentOf(e)) secretUp.add(e);
		}
		/** Node ada di dalam isian rahasia (misalnya anak sebuah editor PIN). */
		const insideMemo = /* @__PURE__ */ new Map();
		const insideSecret = (node) => {
			if (!secrets.size) return false;
			const path = [];
			let found = false;
			for (let e = node.nodeType === Node.ELEMENT_NODE ? node : node.parentElement; e; e = parentOf(e)) {
				if (insideMemo.has(e)) {
					found = insideMemo.get(e);
					break;
				}
				path.push(e);
				if (secrets.has(e)) {
					found = true;
					break;
				}
			}
			for (const e of path) insideMemo.set(e, found);
			return found;
		};
		/** Node adalah, berisi, atau berada di dalam isian rahasia. */
		const touchesSecret = (node) => secrets.has(node) || secretUp.has(node) || insideSecret(node);
		const secretField = (el) => checked.has(el) ? secrets.has(el) : isSecret(el, labelHints);
		/** Teks yang tampil di dalam node, tanpa isian (rahasia atau bukan), tanpa yang tersembunyi. */
		const safeText = (node, max = MAX_TEXT) => {
			if (insideSecret(node)) return "";
			let out = "";
			const visit = (n) => {
				if (out.length > max) return;
				if (n.nodeType === Node.TEXT_NODE) {
					const holder = n.parentElement || n.parentNode?.host;
					if (!holder || getComputedStyle(holder).visibility !== "hidden") out += ` ${n.textContent}`;
					return;
				}
				if (n.nodeType === Node.ELEMENT_NODE) {
					if (secrets.has(n) || CONTROL_TAGS.has(n.tagName) || SKIP_TAGS.has(n.tagName)) return;
					if (n.getAttribute("aria-hidden") === "true" || n.inert || !rendered(n)) return;
				}
				for (const c of childrenOf(n)) visit(c);
			};
			visit(node);
			return clean(out, max);
		};
		/** Teks tampil sebuah blok: innerText kalau aman, selain itu dihitung tanpa isian rahasia dan
		* tanpa bagian aria-hidden. */
		const blockText = (el, max = MAX_TEXT) => touchesSecret(el) || el.querySelector("[aria-hidden=\"true\"]") || shadowUp.has(el) ? safeText(el, max) : clean(el.innerText, max);
		const nameOf = (el, role) => {
			const labelledBy = referenced(el, "aria-labelledby");
			const labels = el.labels ? Array.from(el.labels) : [];
			const owned = referenced(el, "aria-owns");
			if (![
				el,
				...labelledBy,
				...labels,
				...owned
			].some(touchesSecret)) return clean(computeAccessibleName(el));
			if (el.getAttribute("aria-label")) return clean(el.getAttribute("aria-label"));
			if (labelledBy.length) return clean(labelledBy.map((r) => safeText(r)).join(" "), MAX_NAME);
			if (labels.length) return clean(labels.map((l) => safeText(l)).join(" "), MAX_NAME);
			const legend = el.tagName === "FIELDSET" ? el.querySelector(":scope > legend") : null;
			if (legend) return safeText(legend, MAX_NAME);
			if (NAME_FROM_CONTENT.has(role)) return clean([el, ...owned].map((n) => safeText(n, MAX_NAME)).join(" "), MAX_NAME);
			if (TABLES.has(role)) {
				const caption = el.querySelector("caption");
				return caption ? safeText(caption, MAX_NAME) : "";
			}
			return clean(el.getAttribute("title"));
		};
		const descOf = (el) => {
			const targets = referenced(el, "aria-describedby");
			if (targets.some(touchesSecret)) return clean(targets.map((t) => safeText(t)).join(" "), MAX_NAME);
			return clean(computeAccessibleDescription(el));
		};
		/** Nilai isian sebagai teks, `null` kalau berupa centang, atau `undefined` kalau rahasia. */
		const fieldValue = (el, role) => {
			if (CHECKABLE.has(role)) return null;
			if (secretField(el)) return void 0;
			if (el.tagName === "SELECT") return Array.from(el.selectedOptions, (o) => clean(o.text)).join(", ");
			if (role === "listbox") {
				const picked = [];
				const visit = (n) => {
					if (n.nodeType !== Node.ELEMENT_NODE && n.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) return;
					if (n.getAttribute?.("aria-selected") === "true") picked.push(safeText(n, MAX_NAME));
					else for (const c of childrenOf(n)) visit(c);
				};
				for (const c of childrenOf(el)) visit(c);
				return picked.join(", ");
			}
			if (typeof el.value === "string" && el.tagName !== "BUTTON") return el.value;
			if (el.isContentEditable) return el.innerText;
			return el.getAttribute("aria-valuetext") ?? el.getAttribute("aria-valuenow") ?? safeText(el);
		};
		const lines = [];
		let used = 0;
		let cut = false;
		const emit = (depth, text) => {
			if (cut) return false;
			const line = `${"  ".repeat(depth)}- ${text}`;
			const id = pendingRef;
			pendingRef = void 0;
			if (used + line.length + 1 > 59948) {
				if (id) refs.delete(id);
				cut = true;
				return false;
			}
			if (id) refEnds.push([id, used + line.length]);
			lines.push(line);
			used += line.length + 1;
			return true;
		};
		const stats = {
			fields: 0,
			actions: 0,
			tables: 0,
			rows: 0,
			messages: 0
		};
		const labelIds = /* @__PURE__ */ new Set();
		for (const el of deepAll("[aria-labelledby], [aria-describedby]")) {
			if (!FIELDS.has(roleOf(el)) || !rendered(el)) continue;
			const ids = `${el.getAttribute("aria-labelledby") || ""} ${el.getAttribute("aria-describedby") || ""}`;
			for (const id of ids.split(/\s+/)) if (id) labelIds.add(id);
		}
		const shownMessages = /* @__PURE__ */ new Set();
		for (const el of deepAll("[role=\"alert\"], [role=\"status\"], [aria-live=\"assertive\"]")) {
			if (!rendered(el) || hiddenUp(el)) continue;
			const text = blockText(el);
			if (!text) continue;
			if (emit(0, `${getRole(el) || "pesan"}: ${quote(text)}`)) stats.messages += 1;
			shownMessages.add(el);
		}
		const cellText = (cell) => {
			const text = blockText(cell, MAX_NAME);
			if (text) return text;
			const input = Array.from(cell.querySelectorAll(FIELD_SELECTOR)).find((i) => rendered(i) && !i.closest("[aria-hidden=\"true\"]"));
			if (!input) return "";
			const role = roleOf(input);
			const v = fieldValue(input, role);
			if (v === void 0) return "(rahasia)";
			return v === null ? states(input, role).join(" ") : clean(v);
		};
		function table(el, role, depth) {
			stats.tables += 1;
			const name = nameOf(el, role);
			const owner = (r) => r.closest("[role=\"grid\"], [role=\"treegrid\"], [role=\"table\"], table");
			const cellsOf = (r) => Array.from(r.querySelectorAll("[role=\"gridcell\"], [role=\"cell\"], [role=\"columnheader\"], [role=\"rowheader\"], td, th")).filter((c) => c.closest("[role=\"row\"], tr") === r && rendered(c) && !c.closest("[aria-hidden=\"true\"]"));
			const columnHeader = (c) => c.getAttribute("role") === "columnheader" || c.tagName === "TH" && !c.getAttribute("role") && c.getAttribute("scope") !== "row";
			const head = [];
			const body = [];
			for (const r of el.querySelectorAll("[role=\"row\"], tr")) {
				if (owner(r) !== el || !rendered(r) || r.closest("[aria-hidden=\"true\"]")) continue;
				const cells = cellsOf(r);
				(r.closest("thead") !== null || cells.length > 0 && cells.every(columnHeader) ? head : body).push({
					r,
					cells
				});
			}
			const rowIndex = (r) => Number(r.getAttribute("aria-rowindex")) || 0;
			let headMax = head.length;
			for (const h of head) headMax = Math.max(headMax, rowIndex(h.r));
			let first = Infinity;
			let last = 0;
			for (const b of body) {
				const n = rowIndex(b.r);
				if (n > 0) {
					first = Math.min(first, n);
					last = Math.max(last, n);
				}
			}
			const total = Number(el.getAttribute("aria-rowcount")) - head.length;
			let range = `${body.length} baris`;
			if (last > 0 && total > 0) range = `baris ${first - headMax}–${last - headMax} dari ${total} terlihat`;
			else if (total > body.length) range = `${body.length} dari ${total} baris terlihat`;
			if (!emit(depth, `${role}${name ? ` ${quote(name)}` : ""} ${ref(el)} ${range}`)) return;
			for (const h of head) emit(depth + 1, `kolom: ${h.cells.map((c) => blockText(c, 60) || "-").join(" | ")}`);
			for (const { r, cells } of body.slice(0, MAX_ROWS)) {
				const n = rowIndex(r) - headMax;
				const st = states(r, "row");
				const label = `baris${n > 0 ? ` ${n}` : ""} ${ref(r)}${st.length ? ` ${st.join(" ")}` : ""}`;
				if (!emit(depth + 1, `${label}: ${cells.map(cellText).join(" | ")}`)) return;
				stats.rows += 1;
			}
			if (body.length > MAX_ROWS) emit(depth + 1, `… ${body.length - MAX_ROWS} baris lagi tidak ditampilkan`);
		}
		function element(el, depth) {
			if (cut || SKIP_TAGS.has(el.tagName) || shownMessages.has(el)) return;
			if (el.getAttribute("aria-hidden") === "true" || el.inert) return;
			if (!rendered(el)) return;
			if (el.tagName === "IFRAME") {
				emit(depth, `frame ${quote(clean(el.title || el.name || "tanpa judul"))}`);
				return;
			}
			if (el.tagName === "LABEL" && el.control) {
				if (el.contains(el.control)) element(el.control, depth);
				return;
			}
			const role = roleOf(el);
			if (el.id && labelIds.has(el.id) && !role && !el.querySelector(MEANINGFUL) && !shadowUp.has(el)) return;
			const visible = getComputedStyle(el).visibility !== "hidden";
			if (visible && el.tagName === "DETAILS" && !el.open) {
				const summary = el.querySelector(":scope > summary");
				emit(depth, `bagian ${quote(summary ? blockText(summary, MAX_NAME) || "detail" : "detail")} ${ref(el)} tertutup`);
				return;
			}
			if (visible && FIELDS.has(role)) {
				const name = nameOf(el, role);
				const value = fieldValue(el, role);
				const st = states(el, role);
				if (value === "") st.push("kosong");
				let text = `${role}${name ? ` ${quote(name)}` : ""} ${ref(el)}`;
				if (st.length) text += ` ${st.join(" ")}`;
				if (value === void 0) text += " = (rahasia, tidak dibaca)";
				else if (value) text += ` = ${quote(clean(value))}`;
				const desc = descOf(el);
				if (desc && desc !== name) text += ` — ${quote(desc)}`;
				if (emit(depth, text)) stats.fields += 1;
				return;
			}
			if (visible && role && ACTIONS.has(role)) {
				const name = nameOf(el, role) || blockText(el, MAX_NAME);
				if (!name) return;
				const st = states(el, role);
				if (emit(depth, `${role} ${quote(name)} ${ref(el)}${st.length ? ` ${st.join(" ")}` : ""}`)) stats.actions += 1;
				return;
			}
			if (visible && role === "heading") {
				const level = el.getAttribute("aria-level") || el.tagName.match(/^H(\d)$/)?.[1];
				emit(depth, `heading ${quote(nameOf(el, role))}${level ? ` [level ${level}]` : ""}`);
				return;
			}
			if (visible && role === "img") {
				const name = nameOf(el, role);
				if (name) emit(depth, `img ${quote(name)}`);
				return;
			}
			if (role && TABLES.has(role)) {
				table(el, role, depth);
				return;
			}
			if (role && MESSAGES.has(role)) {
				const text = blockText(el);
				if (text) emit(depth, `${role}: ${quote(text)}`);
				return;
			}
			let inner = depth;
			if (visible && scrollBox(el)) {
				const name = role ? nameOf(el, role) : "";
				emit(depth, `wadah gulir${name ? ` ${quote(name)}` : ""} ${ref(el)}`);
				inner = depth + 1;
			} else if (visible && role && (ALWAYS.has(role) || IF_NAMED.has(role))) {
				const name = nameOf(el, role);
				if (name || ALWAYS.has(role)) {
					emit(depth, `${role}${name ? ` ${quote(name)}` : ""}`);
					inner = depth + 1;
				}
			}
			if (!el.shadowRoot && el.tagName !== "SLOT" && !shadowUp.has(el) && !el.querySelector(MEANINGFUL)) {
				const raw = visible ? blockText(el, 2001) : "";
				if (raw.length <= MAX_TEXT || !el.children.length) {
					const text = clean(raw, MAX_TEXT);
					if (text) emit(inner, `text: ${quote(text)}`);
					return;
				}
			}
			walk(el, inner);
		}
		function walk(node, depth) {
			for (const child of childrenOf(node)) {
				if (cut) return;
				if (child.nodeType === Node.ELEMENT_NODE) element(child, depth);
				else if (child.nodeType === Node.TEXT_NODE) {
					const holder = child.parentElement || node;
					const text = clean(child.textContent, MAX_TEXT);
					if (text && holder.nodeType === Node.ELEMENT_NODE && getComputedStyle(holder).visibility !== "hidden") emit(depth, `text: ${quote(text)}`);
				}
			}
		}
		const scope = afterScroll ? globalThis.__appScope?.deref() : null;
		if (scope?.isConnected && scope !== document.scrollingElement) element(scope, 0);
		else if (document.body) walk(document.body, 0);
		const text = lines.join("\n") + (cut ? NOTE : "");
		return {
			title: clean(document.title) || location.hostname,
			url: location.href,
			text,
			refEnds,
			stats: {
				...stats,
				refs: seq,
				chars: text.length,
				truncated: cut
			}
		};
	}
	exports.act = act;
	exports.readPage = readPage;
	exports.unwatchFrame = unwatchFrame;
	exports.watchFrame = watchFrame;
	return exports;
})({});
