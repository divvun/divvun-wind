//! Resolve labels using the calling Explorer thread's UI language preferences.
use wind_common::locale::LocalizedText;
use windows_sys::Win32::Globalization::{
    GetThreadPreferredUILanguages, MUI_LANGUAGE_NAME, MUI_MERGE_SYSTEM_FALLBACK,
    MUI_MERGE_USER_FALLBACK,
};

pub fn resolve(text: &LocalizedText) -> Option<&str> {
    // Fixed abbreviations need no Win32 call. Translations are selected at query
    // time: the DLL worker's language is not necessarily the shell UI's language.
    if let LocalizedText::Literal(value) = text {
        return Some(value);
    }
    let languages = preferred_ui_languages();
    text.resolve(languages.iter().map(String::as_str))
}

fn preferred_ui_languages() -> Vec<String> {
    // https://learn.microsoft.com/windows/win32/api/winnls/nf-winnls-getthreadpreferreduilanguages
    // The merged list includes thread, process, user and system UI fallbacks.
    let flags = MUI_LANGUAGE_NAME | MUI_MERGE_SYSTEM_FALLBACK | MUI_MERGE_USER_FALLBACK;
    unsafe {
        // Retry if the preference list grows between the size and data calls.
        for _ in 0..3 {
            let mut count = 0;
            let mut size = 0;
            if GetThreadPreferredUILanguages(flags, &mut count, std::ptr::null_mut(), &mut size)
                == 0
                || size == 0
            {
                break;
            }
            let mut buffer = vec![0u16; size as usize];
            if GetThreadPreferredUILanguages(flags, &mut count, buffer.as_mut_ptr(), &mut size) != 0
            {
                return buffer
                    .split(|&c| c == 0)
                    .take_while(|tag| !tag.is_empty())
                    .filter_map(|tag| String::from_utf16(tag).ok())
                    .collect();
            }
        }
    }
    // LocalizedText still provides the explicit English fallback on API failure.
    Vec::new()
}
