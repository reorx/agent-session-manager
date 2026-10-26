//! Favorites actions, including confirmation before trashing the last remaining copy.
use gpui_kit::*;
use gpui_kit::component::WindowExt;
use agent_session_core::{config, favorites};
use crate::{AppRoot, StoreRequest};

#[derive(Clone, Debug, PartialEq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = app, no_json)]
pub struct FavoriteSession { pub id: String }

impl AppRoot {
    pub(crate) fn toggle_favorites_only(&mut self, cx: &mut Context<Self>) {
        let mut filter = self.current_filter(cx); filter.favorites_only = !filter.favorites_only;
        self.apply_filter(filter, cx);
    }
    pub(crate) fn toggle_favorite(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(row) = &self.selected { let id = row.id.to_string(); self.favorite_session(&id, window, cx); }
    }
    pub(crate) fn favorite_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.table.read(cx).delegate().rows.iter().find(|r| r.id == id).cloned();
        let Some(row) = row else { return; };
        if row.transcript_path.is_none() { window.push_notification("无法收藏：没有 transcript", cx); return; }
        if !row.favorite { self.store.request(StoreRequest::Favorite(id.into())); return; }
        if favorites::original_exists(&self.claude, &row.project_key, id) {
            self.store.request(StoreRequest::Unfavorite(id.into())); return;
        }
        let id = id.to_string();
        let answer = window.prompt(PromptLevel::Warning, "这是此会话的最后一份副本", Some("原文件已被清理。取消收藏会把保存的副本移到废纸篓。"), &["移到废纸篓", "取消"], cx);
        cx.spawn_in(window, async move |this, cx| {
            let confirmed = answer.await == Ok(0);
            this.update_in(cx, |this, window, cx| {
                if confirmed { this.store.request(StoreRequest::Unfavorite(id)); }
                this.focus_table(window, cx);
            }).ok();
        }).detach();
    }
    pub(crate) fn change_favorites_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if config::environment_override() { window.push_notification("收藏目录由环境变量 ASM_FAVORITES_DIR 指定", cx); return; }
        let response = cx.prompt_for_paths(PathPromptOptions { directories: true, files: false, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            let result = response.await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Ok(Some(paths))) => { if let Some(path) = paths.first() { this.store.request(StoreRequest::SetFavoritesDir(Some(path.clone()))); } }
                    Ok(Err(error)) => window.push_notification(format!("{error}"), cx),
                    _ => {}
                }
                this.focus_table(window, cx);
            }).ok();
        }).detach();
    }
    pub(crate) fn default_favorites_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if config::environment_override() { window.push_notification("收藏目录由环境变量 ASM_FAVORITES_DIR 指定", cx); return; }
        self.store.request(StoreRequest::SetFavoritesDir(None));
    }
}

impl AppRoot {
    /// Isolated acceptance driver; never sends OS keys or touches the clipboard.
    pub(crate) fn favorites_autoplay(&self, window: &Window, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..300 {
                cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                if this.update_in(cx, |this, _, cx| matches!(this.index_status, crate::IndexStatus::Idle(_)) && !this.table.read(cx).delegate().ix.is_empty()).unwrap_or(false) { break; }
            }
            this.update_in(cx, |this, window, cx| {
                let requested = std::env::var("ASM_AUTOPLAY_SESSION").ok();
                let delegate = this.table.read(cx).delegate();
                let ix = (0..delegate.ix.len()).find(|i| {
                    let row = delegate.row(*i);
                    row.transcript_path.is_some() && requested.as_ref().is_none_or(|id| row.id.starts_with(id))
                });
                if let Some(ix) = ix {
                    this.select_row(ix, cx);
                    if !this.selected.as_ref().unwrap().favorite { window.dispatch_action(Box::new(crate::ToggleFavorite), cx); }
                }
            }).ok();
            for _ in 0..300 {
                cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                if this.update_in(cx, |this, _, _| this.selected.as_ref().is_some_and(|r| r.favorite)).unwrap_or(false) { break; }
            }
            this.update_in(cx, |this, _, cx| {
                println!("autoplay-favorites: saved");
                cx.notify();
                let _ = this;
            }).ok();
            cx.background_executor().timer(std::time::Duration::from_secs(3)).await;
            this.update_in(cx, |this, _, cx| {
                if !this.current_filter(cx).favorites_only { this.toggle_favorites_only(cx); }
                println!("autoplay-favorites: filtered");
            }).ok();
            cx.background_executor().timer(std::time::Duration::from_secs(3)).await;
            if let Ok(query) = std::env::var("ASM_AUTOPLAY_QUERY") {
                this.update_in(cx, |this, window, cx| this.set_query(&query, window, cx)).ok();
                cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
                println!("autoplay-favorites: searched");
            }
        })
    }
}
