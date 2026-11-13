//! Agent gestures and node-scoped form entry.
//!
//! Points that the agent gives are pixels of the screenshot it observed. The
//! driver divides them by the page's device pixel ratio to get the CSS
//! pixels that `DevTools` input uses (ADR-0065, decision 6).

use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::driver::{Chrome, Context, Driver, State, Viewport};
use super::observation::{ObservedElement, observed_element};
use super::{ChromeBatchFailure, ChromeError, ChromeInput, ChromeTarget, lock};

const MAX_INPUT_BYTES: usize = 16_384;
const MAX_PROTECTED_VALUES: usize = 32;

const FILL_FIELD: &str = "function (value, allowPassword, allowTextarea, protectedOrigin) {
    const atExpectedOrigin = () => protectedOrigin === null
        || (this.ownerDocument === document && location.origin === protectedOrigin);
    if (!atExpectedOrigin()) return false;
    let prototype;
    if (this instanceof HTMLInputElement) {
        if (!['text', 'email', 'password', 'search', 'tel', 'url', 'number'].includes(this.type)) return false;
        if (this.type === 'password' && !allowPassword) return false;
        prototype = HTMLInputElement.prototype;
    } else if (this instanceof HTMLTextAreaElement && allowTextarea) {
        prototype = HTMLTextAreaElement.prototype;
    } else {
        return false;
    }
    if (!this.isConnected || this.disabled || this.readOnly || !this.getClientRects().length) return false;
    HTMLElement.prototype.focus.call(this);
    if (!this.isConnected || this.disabled || this.readOnly || !atExpectedOrigin()) return false;
    Object.getOwnPropertyDescriptor(prototype, 'value').set.call(this, value);
    EventTarget.prototype.dispatchEvent.call(this, new Event('input', { bubbles: true, composed: true }));
    EventTarget.prototype.dispatchEvent.call(this, new Event('change', { bubbles: true }));
    return { applied: true };
}";

const SELECT_OPTION: &str = "function (value) {
    if (!(this instanceof HTMLSelectElement) || !this.isConnected || this.disabled || !this.getClientRects().length) return false;
    if (!Array.prototype.some.call(this.options, option => option.value === value && !option.disabled)) return false;
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(this, value);
    EventTarget.prototype.dispatchEvent.call(this, new Event('input', { bubbles: true, composed: true }));
    EventTarget.prototype.dispatchEvent.call(this, new Event('change', { bubbles: true }));
    return { applied: true };
}";

impl Chrome {
    /// Clicks elements in order under one observed document and control revision.
    /// A stopped batch reports its progress and never retries a sent click.
    pub async fn click_many(
        &self,
        tab: &str,
        revision: &str,
        references: &[String],
    ) -> Result<(), ChromeBatchFailure> {
        let start_error = |error| ChromeBatchFailure {
            completed: 0,
            failed_index: 0,
            error,
        };
        let mut context = self
            .inner
            .context(tab, Some(revision))
            .map_err(start_error)?;
        let targets = {
            let state = lock(&self.inner.state);
            context.batch_tab_generation = Some(state.tab_generation);
            let page = state
                .tabs
                .get(tab)
                .ok_or_else(|| start_error(ChromeError::NoTab))?;
            if !page.is_current(&context) || !page.settled() {
                return Err(start_error(ChromeError::Stale));
            }
            references
                .iter()
                .enumerate()
                .map(|(index, reference)| {
                    page.elements
                        .get(reference)
                        .copied()
                        .ok_or(ChromeBatchFailure {
                            completed: 0,
                            failed_index: index,
                            error: ChromeError::Stale,
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
        };

        for (index, target) in targets.into_iter().enumerate() {
            self.inner
                .batch_click(&context, target)
                .await
                .map_err(|error| ChromeBatchFailure {
                    completed: index,
                    failed_index: index,
                    error,
                })?;
        }
        Ok(())
    }

    /// Sends one validated agent gesture.
    pub async fn interact(
        &self,
        tab: &str,
        revision: &str,
        action: ChromeInput,
    ) -> Result<(), ChromeError> {
        let context = self.inner.context(tab, Some(revision))?;
        self.inner.begin_input(&context, true)?;
        match action {
            ChromeInput::Click(target) => self.inner.click(&context, target).await?,
            ChromeInput::Drag { from, to } => self.inner.drag(&context, &from, &to).await?,
            ChromeInput::Fill {
                target,
                text,
                protected,
            } => self.inner.fill(&context, target, text, protected).await?,
            ChromeInput::Select { target, value } => {
                self.inner.select(&context, target, value).await?;
            }
            ChromeInput::Press { key } => self.inner.press(&context, &key).await?,
            ChromeInput::Scroll { x, y } => self.inner.scroll(&context, x, y).await?,
        }
        self.finish_input(&context).await
    }

    /// Fills a protected value, then returns the navigation settlement. The
    /// caller can release the credential lease once the field has received it.
    pub async fn dispatch_credential(
        &self,
        tab: &str,
        revision: &str,
        origin: &str,
        target: ChromeTarget,
        value: String,
    ) -> Result<impl Future<Output = Result<(), ChromeError>> + use<'_>, ChromeError> {
        let context = self.inner.context(tab, Some(revision))?;
        if context.origin.as_deref() != Some(origin) {
            return Err(ChromeError::CrossOriginTarget);
        }
        self.inner.begin_input(&context, true)?;
        self.inner.fill(&context, target, value, true).await?;
        Ok(async move { self.finish_input(&context).await })
    }

    async fn finish_input(&self, context: &Context) -> Result<(), ChromeError> {
        self.inner
            .settle()
            .await
            .map_err(|error| self.inner.dispatched(context, error))?;
        self.inner.outcome()
    }
}

impl Driver {
    async fn click(&self, context: &Context, target: ChromeTarget) -> Result<(), ChromeError> {
        let (x, y) = match &target {
            ChromeTarget::Point { x, y } => {
                let (point, _) = self.point_target(context, *x, *y).await?;
                point
            }
            ChromeTarget::Element(_) => {
                let backend = self.target_backend(context, &target).await?;
                let (x, y) = self.element_point(context, backend).await?;
                self.validate_hit(context, backend, x, y).await?;
                (x, y)
            }
        };
        self.click_at(context, x, y).await
    }

    /// Drags from one target to another as a person would. With drag
    /// interception on, an HTML drag hands its data to the driver instead of
    /// starting the operating system's drag, and the driver plays the drag
    /// with `DevTools` drag events; a pointer-based library sees the mouse
    /// moves alone.
    async fn drag(
        &self,
        context: &Context,
        from: &ChromeTarget,
        to: &ChromeTarget,
    ) -> Result<(), ChromeError> {
        if matches!(from, ChromeTarget::Element(_)) || matches!(to, ChromeTarget::Element(_)) {
            self.drag_point(context, from, true).await?;
        }
        let end = self.drag_point(context, to, true).await?;
        // Check the start after the end's scroll, which can move it or stale
        // the screenshot coordinates.
        let start = self.drag_point(context, from, false).await?;
        if let Some(page) = lock(&self.state).tabs.get_mut(&context.id) {
            page.drag_data = None;
        }
        self.send(
            context,
            "Input.setInterceptDrags",
            json!({ "enabled": true }),
        )
        .await?;
        let dragged = self.drag_gesture(context, start, end).await;
        let _released = self
            .command(
                context,
                "Input.setInterceptDrags",
                json!({ "enabled": false }),
            )
            .await;
        dragged
    }

    /// Where a drag starts or ends, checked against the live page. An element
    /// is scrolled into view first when `scroll` is set.
    async fn drag_point(
        &self,
        context: &Context,
        target: &ChromeTarget,
        scroll: bool,
    ) -> Result<(i32, i32), ChromeError> {
        match target {
            ChromeTarget::Point { x, y } => {
                let (point, _) = self.point_target(context, *x, *y).await?;
                Ok(point)
            }
            ChromeTarget::Element(_) => {
                let backend = self.target_backend(context, target).await?;
                let (x, y) = if scroll {
                    self.element_point(context, backend).await?
                } else {
                    let quad = self
                        .command(
                            context,
                            "DOM.getContentQuads",
                            json!({ "backendNodeId": backend }),
                        )
                        .await?;
                    center(&quad, self.viewport(context)?)?
                };
                self.validate_hit(context, backend, x, y).await?;
                Ok((x, y))
            }
        }
    }

    async fn drag_gesture(
        &self,
        context: &Context,
        (start_x, start_y): (i32, i32),
        (end_x, end_y): (i32, i32),
    ) -> Result<(), ChromeError> {
        const STEPS: i32 = 12;
        let mouse = |kind: &str, x: i32, y: i32, buttons: u8| {
            json!({
                "type": kind, "x": x, "y": y, "button": "left", "buttons": buttons,
                "clickCount": 1,
            })
        };
        let point = |step: i32| {
            (
                start_x + (end_x - start_x) * step / STEPS,
                start_y + (end_y - start_y) * step / STEPS,
            )
        };
        self.send_with_activity(
            context,
            "Input.dispatchMouseEvent",
            mouse("mousePressed", start_x, start_y, 1),
            Some("drag"),
        )
        .await?;
        // The first moves pass the page's drag threshold, where an HTML drag
        // starts and interception reports it.
        let mut moved = Ok(Value::Null);
        let mut data = None;
        for step in 1..=STEPS {
            let (x, y) = point(step);
            moved = self
                .send(
                    context,
                    "Input.dispatchMouseEvent",
                    mouse("mouseMoved", x, y, 1),
                )
                .await;
            if moved.is_err() {
                break;
            }
            data = self.intercepted_drag(context, step <= 3).await;
            if data.is_some() {
                break;
            }
        }
        let sent = match (moved, data) {
            (Err(error), _) => Err(error),
            (Ok(_), Some(data)) => self.play_drag(context, &data, point(0), point(STEPS)).await,
            (Ok(value), None) => Ok(value),
        };
        let released = self
            .send(
                context,
                "Input.dispatchMouseEvent",
                mouse("mouseReleased", end_x, end_y, 0),
            )
            .await;
        // Every part after the press may already have changed the page.
        sent.and(released)
            .map(|_| ())
            .map_err(|error| self.dispatched(context, error))
    }

    /// Plays an intercepted HTML drag from `start` to `end` with `DevTools`
    /// drag events, so the page sees enter, its own drag-over steps and the
    /// drop.
    async fn play_drag(
        &self,
        context: &Context,
        data: &Value,
        (start_x, start_y): (i32, i32),
        (end_x, end_y): (i32, i32),
    ) -> Result<Value, ChromeError> {
        const STEPS: i32 = 12;
        let drag = |kind: &str, step: i32| {
            json!({
                "type": kind,
                "x": start_x + (end_x - start_x) * step / STEPS,
                "y": start_y + (end_y - start_y) * step / STEPS,
                "data": data,
                "modifiers": 0,
            })
        };
        self.send(context, "Input.dispatchDragEvent", drag("dragEnter", 0))
            .await?;
        for step in 1..=STEPS {
            self.send(context, "Input.dispatchDragEvent", drag("dragOver", step))
                .await?;
        }
        self.send(context, "Input.dispatchDragEvent", drag("drop", STEPS))
            .await
    }

    /// The data of an HTML drag that the press and moves started, if they
    /// started one. Interception reports it as an event shortly after; the
    /// first moves wait for it a little, later ones only look.
    async fn intercepted_drag(&self, context: &Context, wait: bool) -> Option<Value> {
        let tries = if wait { 10 } else { 1 };
        for _ in 0..tries {
            let data = lock(&self.state)
                .tabs
                .get_mut(&context.id)
                .and_then(|page| page.drag_data.take());
            if data.is_some() {
                return data;
            }
            if wait {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
        None
    }

    async fn batch_click(
        &self,
        context: &Context,
        target: ObservedElement,
    ) -> Result<(), ChromeError> {
        self.begin_input(context, true)?;
        let tree = self
            .command(
                context,
                "Accessibility.getFullAXTree",
                json!({ "frameId": context.main_frame }),
            )
            .await?;
        let backend = batch_backend(target, &tree)?;
        let (x, y) = self
            .element_point(context, backend)
            .await
            .map_err(stale_element)?;
        // Scrolling into view can itself change a virtualized or rerendered
        // page. Check this exact node again before sending the click.
        let live = self
            .command(
                context,
                "Accessibility.getPartialAXTree",
                json!({ "backendNodeId": backend, "fetchRelatives": false }),
            )
            .await
            .map_err(stale_element)?;
        if !ax_elements(&live)?.any(|node| node.backend == backend && node.same_identity(target)) {
            return Err(ChromeError::Stale);
        }
        self.validate_hit(context, backend, x, y)
            .await
            .map_err(stale_element)?;
        self.click_at(context, x, y).await?;
        self.settle()
            .await
            .map_err(|error| self.dispatched(context, error))?;
        self.outcome()
    }

    async fn element_point(
        &self,
        context: &Context,
        backend: i64,
    ) -> Result<(i32, i32), ChromeError> {
        self.revalidate(context)?;
        self.send(
            context,
            "DOM.scrollIntoViewIfNeeded",
            json!({ "backendNodeId": backend }),
        )
        .await?;
        let quad = self
            .command(
                context,
                "DOM.getContentQuads",
                json!({ "backendNodeId": backend }),
            )
            .await?;
        let viewport = self.viewport(context)?;
        center(&quad, viewport)
    }

    async fn validate_hit(
        &self,
        context: &Context,
        backend: i64,
        x: i32,
        y: i32,
    ) -> Result<(), ChromeError> {
        let hit = self.hit_test(context, x, y).await?;
        if hit != backend && !self.contains(context, backend, hit).await? {
            return Err(ChromeError::Stale);
        }
        Ok(())
    }

    async fn click_at(&self, context: &Context, x: i32, y: i32) -> Result<(), ChromeError> {
        self.inside(context, x, y)?;
        self.revalidate(context)?;
        self.send_with_activity(
            context,
            "Input.dispatchMouseEvent",
            json!({
                "type": "mousePressed", "x": x, "y": y, "button": "left", "buttons": 1, "clickCount": 1,
            }),
            Some("click"),
        )
        .await?;
        self.send(context, "Input.dispatchMouseEvent", json!({
            "type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": 1,
        })).await.map_err(|error| {
            if context.batch_tab_generation.is_some() {
                // Mouse-down handlers may already have changed the page.
                self.dispatched(context, ChromeError::Uncertain)
            } else {
                error
            }
        })?;
        Ok(())
    }

    async fn fill(
        &self,
        context: &Context,
        target: ChromeTarget,
        text: String,
        protected: bool,
    ) -> Result<(), ChromeError> {
        let text = bounded(text)?;
        let backend = self.target_backend(context, &target).await?;
        let world = self.world(context).await?;
        let node = self.node(context, backend, world).await?;
        self.revalidate(context)?;
        let protected_origin = if protected {
            Some(
                context
                    .origin
                    .as_deref()
                    .ok_or(ChromeError::CrossOriginTarget)?,
            )
        } else {
            None
        };

        if protected {
            let mut state = lock(&self.state);
            let State {
                tabs,
                protected_values,
                ..
            } = &mut *state;
            let page = tabs.get_mut(&context.id).ok_or(ChromeError::NoTab)?;
            if page.tab.revision != context.revision {
                return Err(ChromeError::Stale);
            }
            if !protected_values
                .iter()
                .any(|value| value.as_str() == text.as_str())
            {
                if protected_values.len() >= MAX_PROTECTED_VALUES {
                    return Err(ChromeError::Limit);
                }
                protected_values.push(text.clone());
            }
            page.sensitive_documents.insert(page.document.clone());
            page.screenshot_size = None;
        }

        // Only a protected value may enter a password field, and only an
        // unprotected one a text area, where a page would display it.
        let result = self
            .call_on(
                context,
                &node,
                FILL_FIELD,
                json!([
                    { "value": text.as_str() }, { "value": protected }, { "value": !protected },
                    { "value": protected_origin },
                ]),
            )
            .await?;
        self.finish_form_action(context, &result, "fill")
    }

    async fn select(
        &self,
        context: &Context,
        target: ChromeTarget,
        value: String,
    ) -> Result<(), ChromeError> {
        let value = bounded(value)?;
        let backend = self.target_backend(context, &target).await?;
        let world = self.world(context).await?;
        let node = self.node(context, backend, world).await?;
        self.revalidate(context)?;
        let result = self
            .call_on(
                context,
                &node,
                SELECT_OPTION,
                json!([{ "value": value.as_str() }]),
            )
            .await?;
        self.finish_form_action(context, &result, "select")
    }

    fn finish_form_action(
        &self,
        context: &Context,
        result: &Value,
        kind: &'static str,
    ) -> Result<(), ChromeError> {
        if result["result"]["value"]["applied"].as_bool() != Some(true) {
            return Err(ChromeError::Rejected);
        }
        self.record_activity(context, kind);
        Ok(())
    }

    fn record_activity(&self, context: &Context, kind: &'static str) {
        let mut state = lock(&self.state);
        if let Some(page) = state.tabs.get_mut(&context.id)
            && page.is_current(context)
        {
            page.record_activity(kind);
        }
    }

    async fn target_backend(
        &self,
        context: &Context,
        target: &ChromeTarget,
    ) -> Result<i64, ChromeError> {
        match target {
            ChromeTarget::Point { x, y } => {
                let (_, backend) = self.point_target(context, *x, *y).await?;
                Ok(backend)
            }
            ChromeTarget::Element(reference) => {
                let state = lock(&self.state);
                let page = state.tabs.get(&context.id).ok_or(ChromeError::NoTab)?;
                page.elements
                    .get(reference)
                    .map(|element| element.backend)
                    .ok_or(ChromeError::Stale)
            }
        }
    }

    /// The viewport that the agent's latest observation of the tab measured.
    pub(super) fn viewport(&self, context: &Context) -> Result<Viewport, ChromeError> {
        let state = lock(&self.state);
        let page = state.tabs.get(&context.id).ok_or(ChromeError::NoTab)?;
        if page.viewport.width <= 0.0 || page.viewport.height <= 0.0 {
            return Err(ChromeError::Stale);
        }
        Ok(page.viewport)
    }

    fn inside(&self, context: &Context, x: i32, y: i32) -> Result<(), ChromeError> {
        if !self.viewport(context)?.contains(f64::from(x), f64::from(y)) {
            return Err(ChromeError::Stale);
        }
        Ok(())
    }

    /// Maps a screenshot point to the live page, using one viewport measurement
    /// for both the input coordinates and the document hit test.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the viewport and document offsets are bounded before conversion"
    )]
    async fn point_target(
        &self,
        context: &Context,
        x: i32,
        y: i32,
    ) -> Result<((i32, i32), i64), ChromeError> {
        let expected = {
            let state = lock(&self.state);
            let page = state.tabs.get(&context.id).ok_or(ChromeError::NoTab)?;
            if !page.is_current(context) || !page.settled() {
                return Err(ChromeError::Stale);
            }
            let (width, height) = page
                .screenshot_size
                .ok_or(ChromeError::ScreenshotRequired)?;
            if !u32::try_from(x).is_ok_and(|x| x < width)
                || !u32::try_from(y).is_ok_and(|y| y < height)
            {
                return Err(ChromeError::PointOutOfBounds);
            }
            page.viewport
        };
        let viewport = self.layout(context).await.map_err(|error| match error {
            ChromeError::Stale => ChromeError::ViewportChanged,
            other => other,
        })?;
        if viewport != expected {
            return Err(ChromeError::ViewportChanged);
        }
        let x = f64::from(x) / viewport.scale;
        let y = f64::from(y) / viewport.scale;
        if !viewport.contains(x, y) {
            return Err(ChromeError::PointOutOfBounds);
        }
        let x = x.floor();
        let y = y.floor();
        let document_x = (x + viewport.document_x).round() as i32;
        let document_y = (y + viewport.document_y).round() as i32;
        let backend = self.hit_test_at(context, document_x, document_y).await?;
        self.revalidate(context)?;
        Ok(((x as i32, y as i32), backend))
    }

    async fn hit_test(&self, context: &Context, x: i32, y: i32) -> Result<i64, ChromeError> {
        self.inside(context, x, y)?;
        // Input events take viewport coordinates, and the hit test takes the
        // document's: add the page's scroll offset.
        let (scroll_x, scroll_y) = self.scroll_offset(context).await?;
        self.hit_test_at(context, x + scroll_x, y + scroll_y).await
    }

    async fn hit_test_at(&self, context: &Context, x: i32, y: i32) -> Result<i64, ChromeError> {
        let node = self
            .command(context, "DOM.getNodeForLocation", json!({ "x": x, "y": y }))
            .await?;
        if node["frameId"].as_str() != Some(context.main_frame.as_str()) {
            return Err(ChromeError::CrossOriginTarget);
        }
        let backend = node["backendNodeId"].as_i64().ok_or(ChromeError::Stale)?;
        let description = self
            .command(
                context,
                "DOM.describeNode",
                json!({ "backendNodeId": backend }),
            )
            .await?;
        if matches!(
            description["node"]["nodeName"].as_str(),
            Some("IFRAME" | "FRAME" | "OBJECT" | "EMBED")
        ) {
            return Err(ChromeError::CrossOriginTarget);
        }
        Ok(backend)
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "a scroll offset in CSS pixels fits in an i32"
    )]
    async fn scroll_offset(&self, context: &Context) -> Result<(i32, i32), ChromeError> {
        let metrics = self
            .command(context, "Page.getLayoutMetrics", json!({}))
            .await?;
        let offset = |key: &str| {
            metrics["cssLayoutViewport"][key]
                .as_f64()
                .filter(|value| value.is_finite() && value.abs() < 1.0e9)
                .map_or(0, |value| value.round() as i32)
        };
        Ok((offset("pageX"), offset("pageY")))
    }

    async fn contains(
        &self,
        context: &Context,
        ancestor: i64,
        child: i64,
    ) -> Result<bool, ChromeError> {
        let world = self.world(context).await?;
        let ancestor = self.node(context, ancestor, world).await?;
        let child = self.node(context, child, world).await?;
        let result = self.command(context, "Runtime.callFunctionOn", json!({
            "objectId": ancestor,
            "functionDeclaration": "function (child) { return Node.prototype.contains.call(this, child); }",
            "arguments": [{ "objectId": child }], "returnByValue": true,
        })).await?;
        Ok(result["result"]["value"].as_bool() == Some(true))
    }

    async fn world(&self, context: &Context) -> Result<i64, ChromeError> {
        let world = self
            .command(
                context,
                "Page.createIsolatedWorld",
                json!({
                    "frameId": context.main_frame,
                    "worldName": "Mannie",
                }),
            )
            .await?;
        world["executionContextId"]
            .as_i64()
            .ok_or(ChromeError::Stale)
    }

    async fn node(
        &self,
        context: &Context,
        backend: i64,
        world: i64,
    ) -> Result<String, ChromeError> {
        let node = self.command(context, "DOM.resolveNode", json!({
            "backendNodeId": backend, "executionContextId": world, "objectGroup": "mannie-action",
        })).await?;
        node["object"]["objectId"]
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or(ChromeError::Stale)
    }

    async fn call_on(
        &self,
        context: &Context,
        node: &str,
        function: &'static str,
        arguments: Value,
    ) -> Result<Value, ChromeError> {
        let result = self
            .send(
                context,
                "Runtime.callFunctionOn",
                json!({
                    "objectId": node, "functionDeclaration": function, "arguments": arguments,
                    "returnByValue": true, "userGesture": true,
                }),
            )
            .await;
        // A navigation can destroy the group before this release.
        drop(
            self.command(
                context,
                "Runtime.releaseObjectGroup",
                json!({ "objectGroup": "mannie-action" }),
            )
            .await,
        );
        result
    }

    async fn press(&self, context: &Context, key: &str) -> Result<(), ChromeError> {
        let (code, virtual_key) = match key {
            "Enter" => ("Enter", 13),
            "Tab" => ("Tab", 9),
            "Escape" => ("Escape", 27),
            "Space" => ("Space", 32),
            "ArrowLeft" => ("ArrowLeft", 37),
            "ArrowUp" => ("ArrowUp", 38),
            "ArrowRight" => ("ArrowRight", 39),
            "ArrowDown" => ("ArrowDown", 40),
            "Backspace" => ("Backspace", 8),
            "Delete" => ("Delete", 46),
            "Home" => ("Home", 36),
            "End" => ("End", 35),
            _ => return Err(ChromeError::Rejected),
        };
        let world = self.world(context).await?;
        let focus = self.command(context, "Runtime.evaluate", json!({
            "contextId": world,
            "expression": "!document.activeElement || !['IFRAME', 'FRAME', 'OBJECT', 'EMBED'].includes(document.activeElement.tagName)",
            "returnByValue": true,
        })).await?;
        if focus["result"]["value"].as_bool() != Some(true) {
            return Err(ChromeError::CrossOriginTarget);
        }
        self.revalidate(context)?;
        let key = if key == "Space" { " " } else { key };
        for down in [true, false] {
            self.key(context, key, code, virtual_key, down).await?;
        }
        self.record_activity(context, "press");
        Ok(())
    }

    async fn key(
        &self,
        context: &Context,
        key: &str,
        code: &str,
        virtual_key: u16,
        down: bool,
    ) -> Result<(), ChromeError> {
        let kind = match (down, key) {
            (true, "Enter") => "keyDown",
            (true, _) => "rawKeyDown",
            (false, _) => "keyUp",
        };
        let text = if kind == "keyDown" { "\r" } else { "" };
        self.send(
            context,
            "Input.dispatchKeyEvent",
            json!({
                "type": kind, "key": key, "code": code, "modifiers": 0,
                "windowsVirtualKeyCode": virtual_key, "nativeVirtualKeyCode": virtual_key,
                "autoRepeat": false, "text": text,
            }),
        )
        .await?;
        Ok(())
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the center of a measured viewport fits in an i32"
    )]
    async fn scroll(&self, context: &Context, dx: i32, dy: i32) -> Result<(), ChromeError> {
        let viewport = self.viewport(context)?;
        let (x, y) = (
            (viewport.width / 2.0).floor() as i32,
            (viewport.height / 2.0).floor() as i32,
        );
        self.hit_test(context, x, y).await?;
        self.revalidate(context)?;
        self.send_with_activity(
            context,
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel", "x": x, "y": y, "deltaX": dx.clamp(-4000, 4000),
                "deltaY": dy.clamp(-4000, 4000), "modifiers": 0,
            }),
            Some("scroll"),
        )
        .await?;
        Ok(())
    }

    async fn send(
        &self,
        context: &Context,
        method: &str,
        parameters: Value,
    ) -> Result<Value, ChromeError> {
        self.send_with_activity(context, method, parameters, None)
            .await
    }

    async fn send_with_activity(
        &self,
        context: &Context,
        method: &str,
        parameters: Value,
        activity: Option<&'static str>,
    ) -> Result<Value, ChromeError> {
        let call = {
            let mut state = lock(&self.state);
            let generation = state.tab_generation;
            let page = state.tabs.get_mut(&context.id).ok_or(ChromeError::NoTab)?;
            if !page.is_current(context)
                || context
                    .batch_tab_generation
                    .is_some_and(|original| generation != original || !page.settled())
            {
                return Err(ChromeError::Stale);
            }
            // The revision, input queue and reported activity share the
            // state lock.
            let call = self.wire.submit(&context.id, method, parameters)?;
            if let Some(activity) = activity {
                page.record_activity(activity);
            }
            call
        };
        call.wait()
            .await
            .map_err(|error| self.dispatched(context, error))
    }
}

fn ax_elements(tree: &Value) -> Result<impl Iterator<Item = ObservedElement> + '_, ChromeError> {
    let nodes = tree["nodes"].as_array().ok_or(ChromeError::Protocol)?;
    Ok(nodes.iter().filter_map(observed_element))
}

fn batch_backend(target: ObservedElement, tree: &Value) -> Result<i64, ChromeError> {
    let mut elements = ax_elements(tree)?;
    if target.can_rebind {
        // Consider every matching name before comparing checked state. If
        // the site made the label ambiguous, no replacement is safe.
        let mut matches = elements.filter(|node| node.same_name(target));
        let current = matches.next().ok_or(ChromeError::Stale)?;
        if matches.next().is_none() && current.same_identity(target) {
            return Ok(current.backend);
        }
    } else if let Some(current) =
        elements.find(|node| node.backend == target.backend && node.same_identity(target))
    {
        return Ok(current.backend);
    }
    Err(ChromeError::Stale)
}

fn stale_element(error: ChromeError) -> ChromeError {
    if error == ChromeError::Protocol {
        ChromeError::Stale
    } else {
        error
    }
}

fn bounded(value: String) -> Result<Zeroizing<String>, ChromeError> {
    let value = Zeroizing::new(value);
    if value.len() > MAX_INPUT_BYTES {
        return Err(ChromeError::Limit);
    }
    Ok(value)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the validated quad center is rounded to browser pixel coordinates"
)]
fn center(response: &Value, viewport: Viewport) -> Result<(i32, i32), ChromeError> {
    let quad = response["quads"][0]
        .as_array()
        .filter(|quad| quad.len() == 8)
        .ok_or(ChromeError::Stale)?;
    let values: Vec<f64> = quad.iter().filter_map(Value::as_f64).collect();
    if values.len() != 8 {
        return Err(ChromeError::Stale);
    }
    let x = (values[0] + values[2] + values[4] + values[6]) / 4.0;
    let y = (values[1] + values[3] + values[5] + values[7]) / 4.0;
    if !(0.0..viewport.width).contains(&x) || !(0.0..viewport.height).contains(&y) {
        return Err(ChromeError::Stale);
    }
    Ok((x as i32, y as i32))
}
