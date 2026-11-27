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
	/**
	* TODO https://github.com/eps1lon/dom-accessibility-api/issues/100
	*/
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
	// ../extension/src/pointer.js
	var RED = "#365a77";
	/** Tiga denyut, lalu diam 3 detik, berulang sampai disentuh (pengecualian AGENTS.md poin 4). */
	var PULSE_MS = 600;
	var PAUSE_MS = 3e3;
	var GAP = 8;
	var EDGE = 8;
	/** Jarak ujung panah dari sisi kiri balon. */
	var ARROW = 24;
	var SVG = "http://www.w3.org/2000/svg";
	var NO_STROKE = { stroke: "none" };
	var eye = (x) => [
		["circle", {
			cx: x,
			cy: 116,
			r: 21,
			fill: "#fff"
		}],
		["ellipse", {
			cx: x,
			cy: 116,
			rx: 10,
			ry: 13,
			fill: "#188f99",
			...NO_STROKE
		}],
		["ellipse", {
			cx: x + 1,
			cy: 117,
			rx: 5.5,
			ry: 8,
			fill: "#1b2433",
			...NO_STROKE
		}],
		["circle", {
			cx: x - 2,
			cy: 111,
			r: 3.2,
			fill: "#fff",
			...NO_STROKE
		}]
	];
	/** Kepala Bakase dari web/public/brand/app-head.svg, tanpa tulisan kecil di helm. */
	var HEAD = [
		["path", {
			d: "M15 78H115V142Q115 160 65 160Q15 160 15 142Z",
			fill: "#dce4e9"
		}],
		...eye(42),
		...eye(88),
		["path", {
			d: "M63 114Q65 108 67 114M21 113L14 110M109 113L116 110M57 146Q65 153 73 146",
			fill: "none"
		}],
		["path", {
			d: "M10 75C11 43 29 25 57 23H73C101 25 119 43 120 75Z",
			fill: "#fff"
		}],
		["path", {
			d: "M49 54L55 22Q56 17 61 17H69Q74 17 75 22L81 54",
			fill: "#eef1f4"
		}],
		["path", {
			d: "M12 65L43 65V73H10ZM87 65H118L120 73H87Z",
			fill: "#365a77",
			...NO_STROKE
		}],
		["path", {
			d: "M9 77Q65 72 121 77L125 84Q125 89 119 89H11Q5 89 5 84Z",
			fill: "#fff"
		}]
	];
	function node(name, attributes) {
		const el = document.createElementNS(SVG, name);
		for (const [key, value] of Object.entries(attributes)) el.setAttribute(key, String(value));
		return el;
	}
	function head() {
		const svg = node("svg", {
			viewBox: "0 0 148 148",
			width: 25,
			height: 25,
			fill: "none"
		});
		const group = node("g", {
			transform: "translate(9 -14)",
			stroke: "#1b2433",
			"stroke-width": 3.2,
			"stroke-linecap": "round",
			"stroke-linejoin": "round"
		});
		group.append(...HEAD.map(([name, attributes]) => node(name, attributes)));
		svg.append(group);
		return svg;
	}
	var CSS = `
  :host { all: initial; position: fixed; inset: 0 auto auto 0; display: block; margin: 0; padding: 0; border: 0;
    background: none; overflow: visible; pointer-events: none; z-index: 2147483647; }
  .balon { position: fixed; display: flex; align-items: center; gap: 7px; padding: 5px 10px 5px 5px; border: 1px solid #d9d9d9;
    border-radius: 12px; background: #fff; color: #0d0d0d; box-shadow: 0 8px 28px #0003; font: 450 14px/1.4 "Segoe UI", sans-serif;
    white-space: nowrap; pointer-events: auto; cursor: pointer; }
  .balon::before { content: ""; position: absolute; top: -6px; left: 19px; width: 9px; height: 9px; transform: rotate(45deg);
    background: #fff; border: solid #d9d9d9; border-width: 1px 0 0 1px; }
  .balon.atas::before { top: auto; bottom: -6px; border-width: 0 1px 1px 0; }
  .cincin { position: fixed; box-sizing: border-box; border: 2px solid ${RED}; border-radius: 4px; pointer-events: none; }
  .cahaya { position: absolute; inset: -2px; border-radius: 6px; box-shadow: 0 0 18px 7px ${RED}cc; opacity: 0.6; }
`;
	/** Kunci animasi satu putaran: tiga kali terang-redup, lalu diam sampai putaran berikutnya. */
	function pulseFrames(pulses = 3, pulseMs = PULSE_MS, pauseMs = PAUSE_MS) {
		const total = pulses * pulseMs + pauseMs;
		const frames = [{
			opacity: 0,
			offset: 0
		}];
		for (let i = 0; i < pulses; i++) {
			frames.push({
				opacity: 1,
				offset: (i * pulseMs + pulseMs / 2) / total
			});
			frames.push({
				opacity: 0,
				offset: (i + 1) * pulseMs / total
			});
		}
		frames.push({
			opacity: 0,
			offset: 1
		});
		return {
			frames,
			total
		};
	}
	var BUSY_CSS = `
  :host { all: initial; position: fixed; inset: 0; display: block; margin: 0; padding: 0; border: 0;
    background: none; overflow: visible; pointer-events: none; z-index: 2147483646; }
  .tepi { position: fixed; inset: 0; pointer-events: none;
    box-shadow: inset 0 0 0 2px ${RED}, inset 0 0 26px 6px ${RED}99, inset 0 0 80px 18px ${RED}33; }
  .label { position: fixed; left: 50%; bottom: 14px; transform: translateX(-50%); padding: 5px 12px; border-radius: 999px;
    background: #fff; color: #0d0d0d; border: 1px solid ${RED}66; box-shadow: 0 6px 20px #0003;
    font: 500 13px/1.4 "Segoe UI", sans-serif; white-space: nowrap; }
`;
	/**
	* Tanda di tepi layar selama Bakase bekerja di halaman (misalnya mengambil tabel): gradasi merah
	* yang bernapas pelan, diam kalau sistem meminta animasi dikurangi. Mengembalikan fungsi penutup.
	*/
	function busy(text) {
		const host = document.createElement("app-sibuk");
		host.setAttribute("aria-hidden", "true");
		const root = host.attachShadow({ mode: "closed" });
		const sheet = new CSSStyleSheet();
		sheet.replaceSync(BUSY_CSS);
		root.adoptedStyleSheets = [sheet];
		const edge = document.createElement("div");
		edge.className = "tepi";
		const label = document.createElement("div");
		label.className = "label";
		label.textContent = text;
		root.append(edge, label);
		document.documentElement.append(host);
		try {
			host.popover = "manual";
			host.showPopover();
		} catch {}
		const glow = matchMedia("(prefers-reduced-motion: reduce)").matches ? null : edge.animate([{ opacity: .55 }, { opacity: 1 }], {
			duration: 1400,
			iterations: Infinity,
			direction: "alternate"
		});
		return () => {
			glow?.cancel();
			host.remove();
		};
	}
	function clearPointer() {
		globalThis.__appPointer?.();
	}
	/**
	* Menunjuk `el` sampai balon atau elemennya disentuh (klik atau ketuk), halaman dibaca lagi, atau
	* elemennya hilang. Saat kursor di atas balon atau elemen, denyutnya berhenti tetapi penunjuk tetap ada.
	*/
	function point(el) {
		clearPointer();
		const host = document.createElement("app-penunjuk");
		host.setAttribute("aria-hidden", "true");
		const root = host.attachShadow({ mode: "closed" });
		const sheet = new CSSStyleSheet();
		sheet.replaceSync(CSS);
		root.adoptedStyleSheets = [sheet];
		const balloon = document.createElement("div");
		balloon.className = "balon";
		balloon.title = "Klik untuk menyembunyikan penunjuk";
		balloon.append(head(), "Yang ini");
		const ring = document.createElement("div");
		ring.className = "cincin";
		const glow = document.createElement("div");
		glow.className = "cahaya";
		ring.append(glow);
		root.append(ring, balloon);
		document.documentElement.append(host);
		try {
			host.popover = "manual";
			host.showPopover();
		} catch {}
		let frame = 0;
		let visible = true;
		function place() {
			frame = 0;
			const box = el.getBoundingClientRect();
			if (!el.isConnected || !box.width && !box.height) return clear();
			const { offsetWidth: width, offsetHeight: height } = balloon;
			const above = box.bottom + GAP + height > innerHeight && box.top - GAP - height > 0;
			const left = Math.min(Math.max(EDGE, box.left - ARROW / 2), Math.max(EDGE, innerWidth - width - EDGE));
			balloon.classList.toggle("atas", above);
			balloon.style.visibility = ring.style.visibility = visible ? "visible" : "hidden";
			Object.assign(ring.style, {
				left: `${box.left}px`,
				top: `${box.top}px`,
				width: `${box.width}px`,
				height: `${box.height}px`
			});
			balloon.style.left = `${left}px`;
			balloon.style.top = `${above ? box.top - GAP - height : Math.min(box.bottom + GAP, innerHeight - height - EDGE)}px`;
		}
		const move = () => {
			frame ||= requestAnimationFrame(place);
		};
		const watcher = new IntersectionObserver((entries) => {
			visible = entries.at(-1).isIntersecting;
			move();
		});
		watcher.observe(el);
		const row = el.closest("[role=row], tr");
		const seen = el.textContent;
		const seenRow = row?.getAttribute("aria-rowindex");
		const reuse = new MutationObserver(() => {
			if (el.textContent !== seen || row?.getAttribute("aria-rowindex") !== seenRow) clear();
		});
		const rowChange = { attributeFilter: ["aria-rowindex"] };
		reuse.observe(el, {
			characterData: true,
			childList: true,
			subtree: true,
			...row === el && rowChange
		});
		if (row && row !== el) reuse.observe(row, rowChange);
		const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
		const { frames, total } = pulseFrames();
		const pulse = still ? null : glow.animate(frames, {
			duration: total,
			iterations: Infinity
		});
		const hold = () => pulse?.pause();
		const resume = () => pulse?.play();
		function clear() {
			cancelAnimationFrame(frame);
			watcher.disconnect();
			reuse.disconnect();
			pulse?.cancel();
			host.remove();
			el.removeEventListener("click", clear, true);
			el.removeEventListener("pointerenter", hold);
			el.removeEventListener("pointerleave", resume);
			window.removeEventListener("scroll", move, true);
			window.removeEventListener("resize", move);
			if (globalThis.__appPointer === clear) globalThis.__appPointer = null;
		}
		balloon.addEventListener("click", (event) => {
			event.stopPropagation();
			clear();
		});
		balloon.addEventListener("pointerenter", hold);
		balloon.addEventListener("pointerleave", resume);
		el.addEventListener("click", clear, true);
		el.addEventListener("pointerenter", hold);
		el.addEventListener("pointerleave", resume);
		if (el.matches(":hover")) hold();
		window.addEventListener("scroll", move, {
			capture: true,
			passive: true
		});
		window.addEventListener("resize", move);
		globalThis.__appPointer = clear;
		place();
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
	var CELL_SELECTOR = "[role=\"gridcell\"], [role=\"cell\"], [role=\"columnheader\"], [role=\"rowheader\"], td, th";
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
		const table = action === "ambil_tabel";
		if (!Number.isFinite(remaining_ms) || remaining_ms <= 0 || remaining_ms > (table ? 62e3 : 8e3)) return { status: "timeout" };
		const localDeadline = receivedAt + remaining_ms;
		const check = (type, details) => chrome.runtime.sendMessage({
			type,
			id,
			pageId,
			...details
		}).catch(() => false);
		if (!await check("app:izin-aksi") || !await check("app:cek-aksi") || performance.now() >= localDeadline || location.href !== url || document.visibilityState !== "visible") return { status: "unavailable" };
		const entry = ref && globalThis.__appRefs?.get(ref);
		const el = entry?.node.deref();
		if (ref && (!el?.isConnected || !el.getClientRects().length || hiddenUp(el) || el.closest(ROW_SELECTOR)?.getAttribute("aria-rowindex") !== entry.rowIndex || getComputedStyle(el).visibility !== "visible")) return { status: "stale_ref" };
		if (table) {
			if (!el?.matches(TABLE_SELECTOR)) return { status: "stale_ref" };
			const watching = globalThis.__appFrameWatch?.pageId === pageId;
			if (!watching) watchFrame(pageId, url);
			const done = busy("Bakase sedang mengambil tabel");
			try {
				return await collectTable(el, url, Math.min(localDeadline - 750, performance.now() + 6e4), (rows) => check("app:lanjut-tabel", { rows }));
			} finally {
				done();
				if (!watching) unwatchFrame(pageId);
			}
		}
		if (action === "sorot") {
			reveal(el);
			point(el);
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
	function pageContent(scope = document) {
		const labelHints = /* @__PURE__ */ new WeakMap();
		const roots = [scope];
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
		const fields = deepAll(FIELD_SELECTOR);
		for (let el = scope; el.matches; el = parentOf(el) || document) if (el.matches(FIELD_SELECTOR)) fields.push(el);
		for (const el of fields) {
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
		const safeText = (node, max = MAX_TEXT, inCell = false) => {
			if (insideSecret(node)) return "";
			let out = "";
			const visit = (n) => {
				if (out.length > max) return;
				if (n.nodeType === Node.TEXT_NODE) {
					const holder = n.parentElement || n.parentNode?.host;
					if (!holder || getComputedStyle(holder).visibility === "visible") out += inCell ? n.textContent : ` ${n.textContent}`;
					return;
				}
				if (n.nodeType === Node.ELEMENT_NODE) {
					if (secrets.has(n) || CONTROL_TAGS.has(n.tagName) || SKIP_TAGS.has(n.tagName)) return;
					if (n.getAttribute("aria-hidden") === "true" || n.inert || !rendered(n)) return;
					if (inCell && n.tagName === "IMG" && getComputedStyle(n).visibility === "visible") out += n.alt;
				}
				for (const c of childrenOf(n)) visit(c);
			};
			visit(node);
			return inCell ? out : clean(out, max);
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
		const fieldValue = (el, role, full = false) => {
			if (CHECKABLE.has(role)) return null;
			if (insideSecret(el) || secretField(el)) return void 0;
			if (el.tagName === "SELECT") return Array.from(el.selectedOptions, (o) => full ? o.text : clean(o.text)).join(", ");
			if (role === "listbox") {
				const picked = [];
				const visit = (n) => {
					if (n.nodeType !== Node.ELEMENT_NODE && n.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) return;
					if (n.getAttribute?.("aria-selected") === "true") picked.push(full ? safeText(n, CELL_BYTES, true) : safeText(n, MAX_NAME));
					else for (const c of childrenOf(n)) visit(c);
				};
				for (const c of childrenOf(el)) visit(c);
				return picked.join(", ");
			}
			if (typeof el.value === "string" && el.tagName !== "BUTTON") return el.value;
			if (el.isContentEditable) return el.innerText;
			return el.getAttribute("aria-valuetext") ?? el.getAttribute("aria-valuenow") ?? (full ? safeText(el, CELL_BYTES, true) : safeText(el));
		};
		const cellText = (cell, max = MAX_NAME, full = false) => {
			if (insideSecret(cell)) return "(rahasia)";
			const controls = "input, select, textarea, img[alt], [role=\"checkbox\"], [role=\"radio\"], [role=\"switch\"]";
			let value = (touchesSecret(cell) || shadowUp.has(cell) || cell.querySelector(`[aria-hidden="true"], [inert], ${controls}`) ? safeText(cell, max, true) : cell.innerText)?.trim() || "";
			if (!value) {
				const input = Array.from(cell.querySelectorAll(`${FIELD_SELECTOR}, ${controls}`)).find((i) => i.tagName !== "IMG" && shown(i));
				if (input) {
					const role = roleOf(input);
					const v = fieldValue(input, role, full);
					value = v === void 0 ? "(rahasia)" : v === null ? states(input, role).join(" ") : v;
				}
			}
			return full ? value : clean(value, max);
		};
		return {
			deepAll,
			shadowUp,
			touchesSecret,
			blockText,
			nameOf,
			descOf,
			fieldValue,
			cellText
		};
	}
	function shown(el) {
		return rendered(el) && !hiddenUp(el) && getComputedStyle(el).visibility === "visible";
	}
	function tableRows(el) {
		const head = [];
		const body = [];
		for (const r of el.querySelectorAll(ROW_SELECTOR)) {
			if (r.closest(TABLE_SELECTOR) !== el || !shown(r)) continue;
			const cells = Array.from(r.querySelectorAll(CELL_SELECTOR)).filter((c) => c.closest(ROW_SELECTOR) === r && shown(c));
			(r.closest("thead") !== null || cells.length > 0 && cells.every((c) => c.getAttribute("role") === "columnheader" || c.tagName === "TH" && !c.getAttribute("role") && c.getAttribute("scope") !== "row") ? head : body).push({
				r,
				cells
			});
		}
		return {
			head,
			body
		};
	}
	var TABLE_ROWS = 5e3;
	var TABLE_COLUMNS = 100;
	var TABLE_BYTES = 2e6;
	var CELL_BYTES = 8e3;
	var END_WAIT = 850;
	/** Grid yang memuat data dari server saat digulir roda (app) butuh waktu lebih lama. */
	var WHEEL_WAIT = 2500;
	var utf8 = new TextEncoder();
	function positiveIndex(el, attr) {
		const value = Number(el.getAttribute(attr));
		return Number.isSafeInteger(value) && value > 0 ? value : null;
	}
	function clipBytes(text, max) {
		const bytes = utf8.encode(text.slice(0, max));
		if (bytes.length <= max) return text.slice(0, max).toWellFormed();
		let end = max;
		while ((bytes[end] & 192) === 128) end -= 1;
		return new TextDecoder().decode(bytes.subarray(0, end));
	}
	function columnSlots(row, cellText, carried, header, issues) {
		const slots = /* @__PURE__ */ new Map();
		for (const [i, { value, remaining }] of carried) {
			slots.set(i, header ? value : "");
			if (remaining === 1) carried.delete(i);
			else carried.get(i).remaining -= 1;
		}
		let next = positiveIndex(row.r, "aria-colindex") || 1;
		for (const cell of row.cells) {
			while (slots.has(next)) next += 1;
			const start = positiveIndex(cell, "aria-colindex") ?? next;
			const colSpan = positiveIndex(cell, "aria-colspan") || cell.colSpan || 1;
			const rowSpan = positiveIndex(cell, "aria-rowspan") || cell.rowSpan || 1;
			if (start + colSpan - 1 > TABLE_COLUMNS) issues.add("column_limit");
			if (start > TABLE_COLUMNS) continue;
			const full = cellText(cell, CELL_BYTES, true);
			const value = clipBytes(full, CELL_BYTES);
			if (value !== full) issues.add("cell_limit");
			for (let i = start; i < start + colSpan && i <= TABLE_COLUMNS; i += 1) {
				if (slots.has(i)) {
					issues.add("missing_columns");
					continue;
				}
				slots.set(i, i === start || header ? value : "");
				if (rowSpan > 1) carried.set(i, {
					value,
					remaining: rowSpan - 1
				});
			}
			next = start + colSpan;
		}
		return slots;
	}
	function waitForGrid(grid, move, deadline, maxWait) {
		const wait = Math.min(maxWait, deadline - performance.now());
		if (wait <= 0) return Promise.resolve();
		return new Promise((resolve) => {
			let raf;
			let version = 0;
			let previous = -1;
			let quietFrames = 0;
			const observer = new MutationObserver(() => {
				version += 1;
			});
			const stop = () => {
				observer.disconnect();
				cancelAnimationFrame(raf);
				clearTimeout(timer);
				document.removeEventListener("visibilitychange", stop);
				resolve();
			};
			const frame = () => {
				quietFrames = version === previous ? quietFrames + 1 : 0;
				previous = version;
				if (!grid.isConnected || version > 0 && quietFrames >= 2) stop();
				else raf = requestAnimationFrame(frame);
			};
			const timer = setTimeout(stop, wait);
			observer.observe(grid, {
				subtree: true,
				childList: true,
				characterData: true,
				attributes: true
			});
			document.addEventListener("visibilitychange", stop, { once: true });
			move?.();
			raf = requestAnimationFrame(frame);
		});
	}
	/** Area gulir samping milik grid: elemen di antara baris dan grid (atau induknya) yang isinya
	* lebih lebar dari tampilannya. */
	function sideScroller(grid, row) {
		for (let el = row; el; el = parentOf(el)) {
			if (el.clientWidth > 0 && el.scrollWidth > el.clientWidth + 1 && /^(auto|scroll|overlay)$/.test(getComputedStyle(el).overflowX)) return el;
			if (el !== grid && el.contains(grid) && scrollBox(el)) return null;
		}
		return null;
	}
	function typicalHeight(body) {
		const heights = body.filter(({ cells }) => cells.length).map(({ r }) => r.getBoundingClientRect().height);
		return heights.sort((x, y) => x - y)[heights.length >> 1] ?? 0;
	}
	/** Tabel HTML yang hanya merender sebagian barisnya: ada ruang kosong di atas baris pertama atau di
	* bawah baris terakhir (baris atau padding pengganti). */
	function hasSpacer(grid, body) {
		const boxes = body.filter(({ cells }) => cells.length).map(({ r }) => r.getBoundingClientRect());
		if (boxes.length < 3) return false;
		const typical = typicalHeight(body);
		const rows = boxes.filter((b) => b.height <= typical * 3);
		const box = grid.getBoundingClientRect();
		const headBottom = grid.tHead ? grid.tHead.getBoundingClientRect().bottom : box.top;
		const footTop = grid.tFoot?.rows.length ? grid.tFoot.getBoundingClientRect().top : box.bottom;
		const slack = typical * 2 + (grid.caption?.getBoundingClientRect().height ?? 0);
		return rows[0].top - headBottom > slack || footTop - rows.at(-1).bottom > slack;
	}
	async function collectTable(grid, url, deadline, allowed) {
		const issues = /* @__PURE__ */ new Set();
		const rows = /* @__PURE__ */ new Map();
		const headers = [];
		const title = clipBytes(document.title.trim() || location.hostname, 400);
		const initial = tableRows(grid);
		const rowCount = () => positiveIndex(grid, "aria-rowcount");
		const colCount = () => positiveIndex(grid, "aria-colcount");
		let expected = rowCount();
		const declaredColumns = colCount();
		const virtual = grid.tagName !== "TABLE" || ["grid", "treegrid"].includes(grid.getAttribute("role")) || grid.hasAttribute("aria-rowcount") && expected !== initial.body.length + initial.head.length || initial.body.some(({ r }) => getComputedStyle(r).position === "absolute") || hasSpacer(grid, initial.body);
		const scroller = virtual ? scrollerOf(grid) : null;
		const top = scroller?.scrollTop;
		const left = scroller?.scrollLeft;
		const scrollTo = (y) => scroller.scrollTo({
			top: y,
			left: scroller.scrollLeft,
			behavior: "instant"
		});
		const bottom = () => scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 1;
		const running = async () => location.href === url && document.visibilityState === "visible" && grid.isConnected && await allowed(rows.size);
		const needed = Math.min(declaredColumns ?? 0, TABLE_COLUMNS);
		const missesColumns = () => Array.from(rows.values()).some(({ slots }) => slots.size < needed);
		let width = needed;
		let bytes = 0;
		let firstData = initial.head.length + 1;
		let indexed = true;
		let indexedColumns = true;
		let pending = false;
		let stopped = false;
		let slowRenders = 0;
		let wheelMode = false;
		/** Pita layar tempat baris benar-benar tampil: grid, layar, dan wadah baris yang memotong isinya. */
		const band = () => {
			const box = grid.getBoundingClientRect();
			let top = Math.max(0, box.top);
			let end = Math.min(innerHeight, box.bottom);
			const row = tableRows(grid).body.find(({ r }) => positiveIndex(r, "aria-rowindex"))?.r;
			for (let el = row && parentOf(row); el && el !== grid; el = parentOf(el)) {
				if (getComputedStyle(el).overflowY === "visible") continue;
				const clip = el.getBoundingClientRect();
				top = Math.max(top, clip.top);
				end = Math.min(end, clip.bottom);
			}
			return {
				top,
				end
			};
		};
		const inView = (el, view = band()) => {
			const box = el.getBoundingClientRect();
			const middle = (box.top + box.bottom) / 2;
			return box.height > 0 && middle > view.top && middle < view.end;
		};
		const shownIndexes = () => {
			const view = band();
			return tableRows(grid).body.filter(({ r }) => inView(r, view)).map(({ r }) => positiveIndex(r, "aria-rowindex")).filter(Boolean);
		};
		if (declaredColumns > TABLE_COLUMNS) issues.add("column_limit");
		function sample({ head, body }) {
			const count = rowCount();
			if (expected !== null && count > expected) expected = count;
			else if (count !== expected) issues.add("changed");
			if (colCount() !== declaredColumns) issues.add("changed");
			for (const [i, { r }] of head.entries()) firstData = Math.max(firstData, (positiveIndex(r, "aria-rowindex") ?? i + 1) + 1);
			const { cellText } = pageContent(grid);
			const loose = Array.from(grid.querySelectorAll("[role=\"columnheader\"]")).filter((c) => c.closest(TABLE_SELECTOR) === grid && !c.closest(ROW_SELECTOR) && shown(c));
			const currentHeaders = [];
			const headSpans = /* @__PURE__ */ new Map();
			for (const h of loose.length ? [...head, {
				r: grid,
				cells: loose
			}] : head) {
				if (sideLeft !== null && h.cells.some((c) => !positiveIndex(c, "aria-colindex"))) continue;
				const slots = columnSlots(h, cellText, headSpans, true, issues);
				width = Math.max(width, ...slots.keys());
				for (const [i, value] of slots) {
					const parts = currentHeaders[i - 1] ??= [];
					if (value && !parts.includes(value)) parts.push(value);
				}
			}
			for (let i = 0; i < currentHeaders.length; i += 1) {
				const value = currentHeaders[i]?.join(" / ");
				if (value === void 0) continue;
				const clipped = clipBytes(value, CELL_BYTES);
				if (clipped !== value) issues.add("cell_limit");
				if (headers[i] !== void 0 && headers[i] !== clipped) issues.add("changed");
				headers[i] ??= clipped;
			}
			pending = false;
			const rowSpans = /* @__PURE__ */ new Map();
			const positions = /* @__PURE__ */ new Map();
			const portTop = !scroller || scroller === document.scrollingElement ? 0 : scroller.getBoundingClientRect().top;
			const typical = virtual ? typicalHeight(body) : 0;
			const view = wheelMode ? band() : null;
			let previousIndex = null;
			for (const [i, row] of body.entries()) {
				if (performance.now() >= deadline) {
					issues.add("time_limit");
					stopped = true;
					break;
				}
				if (!row.cells.length && !rowSpans.size) continue;
				const index = positiveIndex(row.r, "aria-rowindex") || row.cells[0] && positiveIndex(row.cells[0], "aria-rowindex") || null;
				if (!index) indexed = false;
				if (row.cells.some((cell) => !positiveIndex(cell, "aria-colindex"))) indexedColumns = false;
				if (index !== null && previousIndex !== null && index !== previousIndex + 1) rowSpans.clear();
				previousIndex = index;
				const slots = columnSlots(row, cellText, rowSpans, false, issues);
				width = Math.max(width, ...slots.keys());
				const data = JSON.stringify([...slots]);
				const empty = virtual && Array.from(slots.values()).every((value) => value === "");
				if (empty && row.r.getBoundingClientRect().height > typical * 2.5) continue;
				const position = scroller ? Math.round((row.r.getBoundingClientRect().top - portTop + scroller.scrollTop) * 2) : i;
				const ordinal = positions.get(position) ?? 0;
				positions.set(position, ordinal + 1);
				const sticky = scroller && index === null && /sticky|fixed/.test(getComputedStyle(row.r).position);
				const key = virtual && index !== null ? `i${index}` : sticky ? `s${data}` : `p${position}:${ordinal}`;
				const stored = rows.get(key);
				const awaited = !wheelMode || inView(row.r, view);
				if (stored) {
					for (const [column, value] of empty && wheelMode ? [] : slots) {
						const old = stored.slots.get(column);
						if (old === value) continue;
						if (old !== void 0 && !stored.empty) issues.add("changed");
						stored.slots.set(column, value);
						bytes += utf8.encode(value).length + 3;
					}
					if (!empty) stored.empty = false;
					else if (stored.empty && stored.waited !== true && awaited) pending = true;
					if (bytes > TABLE_BYTES) issues.add("size_limit");
				} else {
					const rowBytes = utf8.encode(data).length + 1;
					if (rows.size === TABLE_ROWS) issues.add("row_limit");
					else if (bytes + rowBytes > TABLE_BYTES) issues.add("size_limit");
					else {
						rows.set(key, {
							index,
							position,
							slots,
							empty
						});
						if (empty && awaited) pending = true;
						bytes += rowBytes;
					}
				}
				if (issues.has("row_limit") || issues.has("size_limit")) {
					stopped = true;
					break;
				}
			}
		}
		/** Grid kadang baru merender setelah beberapa frame (mesin sibuk). Ditunggu sampai baris yang
		* dirender menutup tampilan, supaya tidak ada baris yang terlewat. */
		async function settleView() {
			const until = Math.min(deadline, performance.now() + 2e3);
			const covered = () => {
				const start = scroller === document.scrollingElement ? 0 : scroller.getBoundingClientRect().top;
				const end = Math.min(start + scroller.clientHeight, start + scroller.scrollHeight - scroller.scrollTop);
				const { head, body } = tableRows(grid);
				const under = Math.max(start, ...head.map(({ r }) => r.getBoundingClientRect()).filter((box) => box.top <= start + 4 && box.bottom < end).map((box) => box.bottom));
				const typical = typicalHeight(body);
				const boxes = body.filter(({ r, cells }) => cells.length && !/sticky|fixed/.test(getComputedStyle(r).position)).map(({ r }) => r.getBoundingClientRect()).filter((box) => box.height <= typical * 2.5 && box.bottom > under && box.top < end);
				return boxes.some((box) => box.top <= under + Math.max(4, box.height / 2)) && boxes.some((box) => box.bottom >= end - Math.max(4, box.height / 2));
			};
			while (slowRenders < 3 && !covered()) {
				if (performance.now() >= until) {
					slowRenders += 1;
					return;
				}
				await waitForGrid(grid, null, deadline, Math.max(1, until - performance.now()));
			}
			if (slowRenders < 3) slowRenders = 0;
		}
		/** Satu lintasan dari atas ke bawah. `settle` menunggu baris yang baru dimuat di ujung bawah. */
		async function sweep(settle) {
			if (scroller.scrollTop !== 0) await waitForGrid(grid, () => scrollTo(0), deadline, 180);
			let endedAt = null;
			await settleView();
			while (await running()) {
				const before = rows.size;
				sample(tableRows(grid));
				for (const until = performance.now() + END_WAIT; pending && !stopped && performance.now() < until;) {
					await waitForGrid(grid, null, deadline, Math.max(1, until - performance.now()));
					if (!await running()) return;
					sample(tableRows(grid));
				}
				if (pending) {
					for (const row of rows.values()) if (row.empty) row.waited = true;
				}
				if (stopped) return;
				if (performance.now() >= deadline) {
					issues.add("time_limit");
					stopped = true;
					return;
				}
				if (!bottom()) {
					endedAt = null;
					const from = scroller.scrollTop;
					const step = Math.max(1, Math.floor(scroller.clientHeight * .85));
					await waitForGrid(grid, () => scrollTo(from + step), deadline, 180);
					if (scroller.scrollTop <= from) {
						issues.add("missing_rows");
						return;
					}
					await settleView();
					continue;
				}
				if (!settle) return;
				const now = performance.now();
				if (endedAt === null || rows.size !== before) endedAt = now;
				const waited = now - endedAt;
				if (waited >= END_WAIT && (waited >= END_WAIT * 4 || grid.getAttribute("aria-busy") !== "true")) return;
				await waitForGrid(grid, null, deadline, Math.max(1, END_WAIT - waited));
			}
		}
		/** Tabel biasa di dalam kotak gulir bisa menambah baris saat mentok di bawah. */
		async function growStatic(box) {
			const start = box.scrollTop;
			try {
				let count = tableRows(grid).body.length;
				while (await running()) {
					const until = Math.min(deadline, performance.now() + END_WAIT);
					box.scrollTo({
						top: box.scrollHeight,
						left: box.scrollLeft,
						behavior: "instant"
					});
					while (performance.now() < until && tableRows(grid).body.length === count) await waitForGrid(grid, null, deadline, Math.max(1, until - performance.now()));
					const now = tableRows(grid).body.length;
					if (now === count) return;
					issues.add("unknown_rows");
					count = now;
					if (count > TABLE_ROWS) return;
					if (performance.now() >= deadline) {
						issues.add("time_limit");
						return;
					}
				}
			} finally {
				if (box.isConnected) box.scrollTo({
					top: start,
					left: box.scrollLeft,
					behavior: "instant"
				});
			}
		}
		const rowIndexes = () => tableRows(grid).body.map(({ r }) => positiveIndex(r, "aria-rowindex")).filter(Boolean);
		const colIndexes = () => tableRows(grid).body.flatMap(({ cells }) => cells.map((c) => positiveIndex(c, "aria-colindex"))).filter(Boolean);
		const firstShown = () => Math.min(...shownIndexes());
		const wanted = () => expected - firstData + 1;
		const lastShown = () => Math.max(0, ...shownIndexes());
		const complete = () => rows.size >= wanted() && !Array.from(rows.values()).some((row) => row.empty) && !(needed && missesColumns());
		/** Roda mouse tiruan di baris grid. Tidak menggerakkan kursor, klik, atau memindah fokus. */
		function wheel(dx, dy) {
			(tableRows(grid).body.find(({ r }) => r.getBoundingClientRect().height > 0)?.r ?? grid).dispatchEvent(new WheelEvent("wheel", {
				deltaX: dx,
				deltaY: dy,
				deltaMode: 0,
				bubbles: true,
				cancelable: true
			}));
		}
		const wheelStep = () => {
			const view = band();
			return Math.max(60, Math.floor((view.end - view.top) * .85));
		};
		const wheelSide = () => Math.max(120, Math.floor(grid.getBoundingClientRect().width * .85));
		/** Kembali ke baris pertama semula, dengan langkah sebesar tinggi baris. */
		async function wheelTo(target) {
			for (let i = 0; i < 6 && Number.isFinite(target) && grid.isConnected && location.href === url; i += 1) {
				const now = firstShown();
				if (!Number.isFinite(now) || now === target) return;
				const height = typicalHeight(tableRows(grid).body) || 24;
				await waitForGrid(grid, () => wheel(0, (target - now) * height), deadline + 3e3, 600);
				if (firstShown() === now) return;
			}
		}
		/** Gulir roda sampai ukuran yang dipantau tidak berubah lagi, misalnya baris paling atas. */
		async function wheelUntil(dx, dy, measure, done, extra = 0) {
			for (let i = 0; i < 300 && !done() && grid.isConnected && location.href === url && performance.now() < deadline + extra; i += 1) {
				const before = measure();
				await waitForGrid(grid, () => wheel(dx, dy), deadline + extra, 600);
				if (measure() === before) return;
			}
		}
		/** Grid tanpa area gulir asli (misalnya app) hanya bergerak lewat roda. Baris dikumpulkan per
		* nomor baris dari atas sampai jumlahnya sesuai aria-rowcount. */
		async function wheelSweep() {
			const step = wheelStep();
			await wheelUntil(0, -step * 3, firstShown, () => firstShown() <= firstData);
			let stuck = 0;
			while (await running() && !stopped) {
				sample(tableRows(grid));
				for (const until = performance.now() + WHEEL_WAIT; pending && !stopped && performance.now() < until;) {
					await waitForGrid(grid, null, deadline, Math.max(1, until - performance.now()));
					if (!await running()) return;
					sample(tableRows(grid));
				}
				if (pending) {
					const seen = new Set(shownIndexes());
					for (const row of rows.values()) if (row.empty && seen.has(row.index)) row.waited = true;
				}
				if (stopped || complete() || lastShown() >= expected) return;
				if (performance.now() >= deadline) {
					issues.add("time_limit");
					stopped = true;
					return;
				}
				const before = firstShown();
				const moved = () => Number.isFinite(before) && firstShown() > before;
				await waitForGrid(grid, () => wheel(0, step), deadline, WHEEL_WAIT);
				if (!moved()) await waitForGrid(grid, null, deadline, WHEEL_WAIT);
				if (!moved()) {
					stuck += 1;
					if (stuck >= 3) return;
				} else stuck = 0;
			}
		}
		let side = null;
		let sideLeft = null;
		let wheeled = null;
		try {
			if (!scroller) {
				const box = grid.hasAttribute("aria-rowcount") ? null : scrollerOf(grid);
				if (box && box !== document.scrollingElement && box.scrollHeight > box.clientHeight + 1) await growStatic(box);
				sample(tableRows(grid));
			} else {
				await sweep(true);
				if (indexed && expected !== null && rows.size < expected - firstData + 1 && !stopped) await sweep(true);
				if (indexed && expected !== null && !complete() && !stopped && !grid.contains(scroller) && rowIndexes().length && await running()) {
					wheeled = {
						top: firstShown(),
						left: 0,
						count: rowCount()
					};
					wheelMode = true;
					for (const row of rows.values()) row.waited = false;
					await wheelSweep();
					for (let k = 0; k < 30 && indexedColumns && missesColumns() && !stopped && await running(); k += 1) {
						const before = Math.max(0, ...colIndexes());
						await waitForGrid(grid, () => wheel(wheelSide(), 0), deadline, WHEEL_WAIT);
						if (Math.max(0, ...colIndexes()) <= before) break;
						wheeled.left += 1;
						await wheelSweep();
					}
				}
				for (const { r } of tableRows(grid).body) if (side = sideScroller(grid, r)) break;
				if (side && declaredColumns === null) issues.add("missing_columns");
				if (side && indexed && indexedColumns && missesColumns()) {
					sideLeft = side.scrollLeft;
					const step = Math.max(1, Math.floor(side.clientWidth * .85));
					const last = side.scrollWidth - side.clientWidth;
					for (let x = 0; !stopped && missesColumns(); x = Math.min(x + step, last)) {
						await waitForGrid(grid, () => side.scrollTo({
							left: x,
							behavior: "instant"
						}), deadline, 180);
						await sweep(false);
						if (x >= last || !await running()) break;
					}
				}
			}
			if (!await running()) return { status: "unavailable" };
		} finally {
			if (wheeled && grid.isConnected && location.href === url && rowCount() >= wheeled.count) {
				if (wheeled.left) await wheelUntil(-wheelSide(), 0, () => Math.min(...colIndexes()), () => false, 3e3);
				await wheelTo(wheeled.top);
			}
			if (side?.isConnected && sideLeft !== null) side.scrollTo({
				left: sideLeft,
				behavior: "instant"
			});
			if (scroller?.isConnected && (scroller.scrollTop !== top || scroller.scrollLeft !== left)) scroller.scrollTo({
				top,
				left,
				behavior: "instant"
			});
		}
		const ordered = Array.from(rows.values());
		if (virtual && indexed) ordered.sort((a, b) => a.index - b.index);
		else if (virtual) ordered.sort((a, b) => a.position - b.position);
		if (virtual && (!indexed || expected === null)) issues.add("unknown_rows");
		if (expected !== null && (ordered.length !== expected - firstData + 1 || indexed && ordered.some((row, i) => row.index !== firstData + i)) || ordered.some((row) => row.empty) || grid.getAttribute("aria-busy") === "true") issues.add("missing_rows");
		const blank = /* @__PURE__ */ new Set();
		for (let i = 1; i <= width; i += 1) if (!headers[i - 1]?.trim() && ordered.length && ordered.every(({ slots }) => !slots.get(i))) blank.add(i);
		if (declaredColumns !== null && (missesColumns() || width - blank.size > declaredColumns)) issues.add("missing_columns");
		if (virtual && declaredColumns === null && ordered.some(({ slots }) => slots.size !== width)) issues.add("missing_columns");
		if (!width) {
			width = 1;
			if (virtual) issues.add("missing_columns");
		}
		const kept = Array.from({ length: width }, (_, i) => i + 1).filter((i) => !blank.has(i));
		if (!kept.length) kept.push(1);
		const table = {
			title,
			columns: kept.map((i) => headers[i - 1] ?? `Kolom ${i}`),
			rows: [],
			issues: []
		};
		bytes = utf8.encode(JSON.stringify(table)).length + 256;
		for (const { slots } of ordered) {
			const row = kept.map((i) => slots.get(i) ?? "");
			bytes += utf8.encode(JSON.stringify(row)).length + 1;
			if (bytes > TABLE_BYTES) {
				issues.add("size_limit");
				break;
			}
			table.rows.push(row);
		}
		table.issues = Array.from(issues);
		return {
			status: "table",
			table
		};
	}
	/** Kotak nama Excel Online (alamat sel terpilih) di frame ini, kalau ada. */
	var NAME_BOX = "input#NameBox, input[id*=\"NameBox\" i], input[aria-label=\"Name Box\" i], input[aria-label*=\"Kotak Nama\" i], input[title=\"Name Box\" i]";
	/** Bagian halaman yang sedang difokuskan user di frame ini: sheet dan sel aktif (Excel Online),
	* tab atau posisi yang aktif, isian yang sedang diisi, dan teks yang dipilih. Dicatat bersama
	* hasil Baca halaman supaya Bakase tahu bagian mana yang dimaksud user. Nilai isian rahasia
	* tidak pernah diambil. */
	function readFocus() {
		const items = [];
		const labelHints = /* @__PURE__ */ new WeakMap();
		const name = (el, max = 80) => clean(el.getAttribute("aria-label") || el.innerText || el.textContent || "", max);
		const found = document.querySelector(NAME_BOX);
		const nameBox = found && !isSecret(found, labelHints) ? found : null;
		const tabs = Array.from(document.querySelectorAll("[role=\"tab\"][aria-selected=\"true\"]")).filter(rendered);
		if (nameBox) {
			const sheetList = /sheet|lembar/i;
			const listName = (tab) => {
				const list = tab.closest("[role=\"tablist\"]");
				if (!list) return "";
				const ids = (list.getAttribute("aria-labelledby") || "").split(/\s+/).filter(Boolean);
				return list.getAttribute("aria-label") || ids.map((id) => document.getElementById(id)?.textContent || "").join(" ");
			};
			const lowest = () => tabs.reduce((low, tab) => !low || tab.getBoundingClientRect().top > low.getBoundingClientRect().top ? tab : low, null);
			const sheetTab = tabs.find((tab) => sheetList.test(listName(tab))) || lowest();
			const sheet = sheetTab ? name(sheetTab) : "";
			if (sheet) items.push(`sheet aktif ${quote(sheet)}`);
			const cell = (nameBox.value || "").trim();
			if (/^[\p{L}\p{N}_.$:!' ]{1,60}$/u.test(cell)) items.push(`sel dipilih ${cell}`);
		} else for (const tab of tabs.slice(0, 3)) {
			const text = name(tab);
			if (text) items.push(`tab aktif ${quote(text)}`);
		}
		const current = Array.from(document.querySelectorAll("[aria-current]:not([aria-current=\"false\"])")).filter(rendered);
		for (const el of current.slice(0, 2)) {
			const text = name(el);
			if (text) items.push(`posisi aktif ${quote(text)}`);
		}
		const active = document.activeElement;
		const secret = active ? isSecret(active, labelHints) : false;
		if (active && active !== document.body && active !== document.documentElement && active !== nameBox && rendered(active)) {
			const role = roleOf(active);
			const label = clean(active.labels?.[0]?.innerText || active.getAttribute("aria-label") || active.getAttribute("placeholder") || active.getAttribute("title") || "", 80);
			const field = FIELDS.has(role) || CONTROL_TAGS.has(active.tagName) || active.isContentEditable;
			if (field && !secret && !NOT_FIELD_INPUTS.has((active.type || "").toLowerCase())) {
				const value = clean(active.isContentEditable ? active.innerText : active.value || "", 100);
				items.push(`isian aktif${label ? ` ${quote(label)}` : ""}${value ? ` berisi ${quote(value)}` : ""}`);
			} else if (!field && role && label) items.push(`elemen aktif ${role} ${quote(label)}`);
		}
		const selection = globalThis.getSelection?.();
		const raw = String(selection ?? "");
		const touchesSecret = raw.trim() !== "" && pageContent().deepAll(FIELD_SELECTOR).some((el) => {
			if (!isSecret(el, labelHints)) return false;
			if (selection.rangeCount > 0 && selection.containsNode(el, true)) return true;
			const value = el.isContentEditable ? el.innerText : el.value;
			return typeof value === "string" && value.trim() !== "" && raw.includes(value.trim());
		});
		const selected = secret || touchesSecret ? "" : clean(raw, 300);
		if (selected) items.push(`teks dipilih ${quote(selected)}`);
		return items.map((item) => item.replace(/[\uD800-\uDFFF]/gu, ""));
	}
	/** Fokus hanya tambahan: kalau gagal dibaca, snapshot tetap jalan tanpa fokus. */
	function safeFocus() {
		try {
			return readFocus();
		} catch {
			return [];
		}
	}
	function readPage(prefix = "", base = 0, afterScroll = false) {
		clearPointer();
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
		const { deepAll, shadowUp, touchesSecret, blockText, nameOf, descOf, fieldValue, cellText } = pageContent();
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
		function table(el, role, depth) {
			stats.tables += 1;
			const name = nameOf(el, role);
			const { head, body } = tableRows(el);
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
				if (!emit(depth + 1, `${label}: ${cells.map((cell) => cellText(cell)).join(" | ")}`)) return;
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
			focus: safeFocus(),
			stats: {
				...stats,
				refs: seq,
				chars: text.length,
				truncated: cut
			}
		};
	}
	exports.act = act;
	exports.readFocus = readFocus;
	exports.readPage = readPage;
	exports.unwatchFrame = unwatchFrame;
	exports.watchFrame = watchFrame;
	return exports;
})({});
