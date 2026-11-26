#[cfg(kani)]
mod tests {
    use crate::scheme::{GlobalSchemes, KernelSchemes, SchemeList, SchemeNamespace};
    use alloc::boxed::Box;

    #[kani::proof]
    #[kani::unwind(3)]
    fn verify_scheme_namespace_isolation() {
        let mut list = SchemeList::new();

        let ns1_val: usize = kani::any();
        let ns2_val: usize = kani::any();
        // Assume two distinct namespaces
        kani::assume(ns1_val != ns2_val);

        let ns1 = SchemeNamespace::from(ns1_val);
        let ns2 = SchemeNamespace::from(ns2_val);

        let name = "test_scheme";

        // Insert a scheme into ns1
        // We use a dummy GlobalScheme for testing the SchemeList API
        let scheme = KernelSchemes::Global(GlobalSchemes::Debug);
        let id1 = list
            .insert_and_pass(ns1, name, |id| Ok((scheme, ())))
            .expect("insertion failed")
            .0;

        // The scheme MUST be accessible from its own namespace
        let lookup_ns1 = list.get_name(ns1, name);
        assert!(
            lookup_ns1.is_some(),
            "Scheme should be found in the namespace it was inserted to"
        );
        assert_eq!(lookup_ns1.unwrap().0.get(), id1.get());

        // The scheme MUST NOT be accessible from a different namespace
        // This is the core URL-capability isolation property mapping namespaces to scheme availability
        let lookup_ns2 = list.get_name(ns2, name);
        assert!(
            lookup_ns2.is_none(),
            "Scheme must not be accessible from a different namespace!"
        );
    }

    #[kani::proof]
    #[kani::unwind(3)]
    fn verify_scheme_isolation_on_remove() {
        let mut list = SchemeList::new();
        let ns1 = SchemeNamespace::from(1);

        let name = "test_scheme";
        let scheme = KernelSchemes::Global(GlobalSchemes::Debug);

        // Insert the scheme
        let id1 = list
            .insert_and_pass(ns1, name, |id| Ok((scheme, ())))
            .expect("insertion failed")
            .0;

        // Verify it was correctly inserted
        assert!(list.get_name(ns1, name).is_some());

        // Remove the scheme by ID (can be triggered across namespaces if allowed, but typically by owner)
        list.remove(id1);

        // Capablity MUST no longer be accessible through the name lookup
        assert!(
            list.get_name(ns1, name).is_none(),
            "Removed scheme must not be accessible"
        );
    }
}
