/// Helper macro to convert a literal map into Fields
#[macro_export]
macro_rules! fields {
    ({ $($k:expr => $v:expr),* $(,)? }) => {{
        let mut f = $crate::core::Fields::new();
        $(f.insert($k.into(), $crate::core::Value::from($v));)*
        f
    }};
}

/// Fire-and-forget info event with structured fields
#[macro_export]
macro_rules! event_info_fields {
    ($msg:expr, $fields:expr) => {{
        let f = $fields;
        let msg = $msg.to_string();
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::event::info_fields(msg, f).await;
        });
    }};
}

/// Fire-and-forget warning event with structured fields
#[macro_export]
macro_rules! event_warn_fields {
    ($msg:expr, $fields:expr) => {{
        let f = $fields;
        let msg = $msg.to_string();
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::event::warn_fields(msg, f).await;
        });
    }};
}

/// Fire-and-forget error event with structured fields
#[macro_export]
macro_rules! event_error_fields {
    ($msg:expr, $fields:expr) => {{
        let f = $fields;
        let msg = $msg.to_string();
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::event::error_fields(msg, f).await;
        });
    }};
}

/// Fire-and-forget debug event with structured fields
#[macro_export]
macro_rules! event_debug_fields {
    ($msg:expr, $fields:expr) => {{
        let f = $fields;
        let msg = $msg.to_string();
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::event::debug_fields(msg, f).await;
        });
    }};
}

/// Fire-and-forget scoped info event with structured fields
#[macro_export]
macro_rules! event_info_scoped_fields {
    ($scope:expr, $msg:expr, $fields:expr) => {{
        let scope = $scope.to_string();
        let f = $fields;
        let msg = $msg.to_string();
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::scoped_async(Some(scope), move || async move {
                $crate::runtime::event::info_fields(msg, f).await;
            }).await;
        });
    }};
}

/// Fire-and-forget scoped warning event with structured fields
#[macro_export]
macro_rules! event_warn_scoped_fields {
    ($scope:expr, $msg:expr, $fields:expr) => {{
        let scope = $scope.to_string();
        let f = $fields;
        let msg = $msg.to_string();
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::scoped_async(Some(scope), move || async move {
                $crate::runtime::event::warn_fields(msg, f).await;
            }).await;
        });
    }};
}

/// Fire-and-forget scoped error event with structured fields
#[macro_export]
macro_rules! event_error_scoped_fields {
    ($scope:expr, $msg:expr, $fields:expr) => {{
        let scope = $scope.to_string();
        let f = $fields;
        let msg = $msg.to_string();
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::scoped_async(Some(scope), move || async move {
                $crate::runtime::event::error_fields(msg, f).await;
            }).await;
        });
    }};
}

/// Fire-and-forget scoped debug event with structured fields
#[macro_export]
macro_rules! event_debug_scoped_fields {
    ($scope:expr, $msg:expr, $fields:expr) => {{
        let scope = $scope.to_string();
        let f = $fields;
        let msg = $msg.to_string();
        $crate::runtime::spawn_detached(async move {
            $crate::runtime::scoped_async(Some(scope), move || async move {
                $crate::runtime::event::debug_fields(msg, f).await;
            }).await;
        });
    }};
}
