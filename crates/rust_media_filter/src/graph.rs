//! Filter graph parsing
//!
//! Parses FFmpeg-like filter specification strings into structured filter
//! descriptions. Used by the CLI and can be used programmatically.

use std::collections::HashMap;

/// Parsed filter specification with name and parameters.
///
/// Both named (`key=value`) and positional (just `value`) parameters are
/// supported. Named params are stored in `params`. Positional params (parts
/// without `=`) are stored in `positional` in their original order, and also
/// inserted into `params` with the value `"true"` so flag-style queries
/// (`has_flag`) keep working.
#[derive(Debug, Clone)]
pub struct Filter {
    pub name: String,
    pub params: HashMap<String, String>,
    /// Positional arguments in original order (parts of the filter spec
    /// that don't contain `=`).
    pub positional: Vec<String>,
}

/// Filter graph containing a chain of filters
#[derive(Debug, Clone)]
pub struct FilterGraph {
    pub filters: Vec<Filter>,
}

impl FilterGraph {
    /// Parse a filter graph string in FFmpeg-like format
    /// Format: "filter1=param1=value1:param2=value2,filter2=param=value"
    pub fn parse(filter_str: &str) -> Result<Self, String> {
        let mut filters = Vec::new();

        for filter_spec in filter_str.split(',') {
            let filter_spec = filter_spec.trim();
            if filter_spec.is_empty() {
                continue;
            }

            let filter = Filter::parse(filter_spec)?;
            filters.push(filter);
        }

        if filters.is_empty() {
            return Err("No filters specified".to_string());
        }

        Ok(FilterGraph { filters })
    }
}

impl Filter {
    /// Parse a single filter specification
    /// Format: "filter_name=param1=value1:param2=value2" or "filter_name=val1:val2"
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut parts = spec.splitn(2, '=');
        let name = parts.next().unwrap_or("").trim().to_string();

        if name.is_empty() {
            return Err("Filter name is empty".to_string());
        }

        let mut params = HashMap::new();
        let mut positional = Vec::new();

        if let Some(params_str) = parts.next() {
            let param_parts: Vec<&str> = params_str.split(':').collect();

            for part in param_parts.iter() {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }

                if let Some((key, value)) = part.split_once('=') {
                    params.insert(key.trim().to_string(), value.trim().to_string());
                } else {
                    // Positional / flag-style parameter
                    positional.push(part.to_string());
                    params.insert(part.to_string(), "true".to_string());
                }
            }
        }

        Ok(Filter {
            name,
            params,
            positional,
        })
    }

    /// Get a parameter value
    pub fn get_param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    /// Check if a flag parameter is set
    pub fn has_flag(&self, key: &str) -> bool {
        self.params
            .get(key)
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_filter() {
        let f = Filter::parse("aresample=48000").unwrap();
        assert_eq!(f.name, "aresample");
        assert_eq!(f.params.get("48000").map(|s| s.as_str()), Some("true"));
    }

    #[test]
    fn test_parse_filter_with_kv() {
        let f = Filter::parse("aresample=sample_rate=48000").unwrap();
        assert_eq!(f.name, "aresample");
        assert_eq!(f.get_param("sample_rate"), Some("48000"));
    }

    #[test]
    fn test_parse_filter_graph() {
        let fg = FilterGraph::parse("filter1,filter2=key=val").unwrap();
        assert_eq!(fg.filters.len(), 2);
        assert_eq!(fg.filters[0].name, "filter1");
        assert_eq!(fg.filters[1].name, "filter2");
        assert_eq!(fg.filters[1].get_param("key"), Some("val"));
    }

    #[test]
    fn test_parse_filter_with_flag() {
        let f = Filter::parse("ssim=print_per_frame").unwrap();
        assert!(f.has_flag("print_per_frame"));
    }

    #[test]
    fn test_empty_filter_graph() {
        assert!(FilterGraph::parse("").is_err());
    }

    #[test]
    fn test_positional_args_preserve_order() {
        // scale=640:480 — order matters (W, H)
        let f = Filter::parse("scale=640:480").unwrap();
        assert_eq!(f.name, "scale");
        assert_eq!(f.positional, vec!["640", "480"]);
        // Both are also accessible via params (as flags)
        assert!(f.has_flag("640"));
        assert!(f.has_flag("480"));
    }

    #[test]
    fn test_mixed_positional_and_named() {
        let f = Filter::parse("filter=val1:key=value:val2").unwrap();
        assert_eq!(f.positional, vec!["val1", "val2"]);
        assert_eq!(f.get_param("key"), Some("value"));
    }
}
